//! The system provider: the voices of the computer the host runs on, through the program the operating system gives.
//!
//! - Windows: the voices of the speech API's desktop synthesizer, through Windows PowerShell (`System.Speech`), which every
//!   Windows has. It synthesizes a sentence at a time into memory and writes it, so audio starts after the first sentence.
//! - macOS: `say`, which writes the whole text to a file the program then prints.
//! - Linux and the rest: `espeak-ng` (or `espeak`), which prints a WAV stream. Speech-dispatcher's own client plays on the
//!   host's speakers and hands nothing back, so it cannot be a source of audio for a client elsewhere.
//!
//! The text stays on the computer: nothing here makes a network call. A system whose voices are online ones says so itself.

use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;
use core::time::Duration;

use coqui_acp_api::{Environment, ProcessSpawner, SpawnSpec};
use guatiao::library::{Object, ProviderError};
use guatiao::{FromValue, Schema, ToValue};
use guatiao_intake::Form;

use super::spoken::{self, Head, Spoken};
use crate::speech::output::{
    Leaves, OutputInfo, SayRequest, SpeechOutput, SpeechUtterance, SpeedRange, Voice, VoiceList,
    VoiceQuery, checked_text, sentences, unsupported,
};
use crate::speech::{AudioFormat, PCM_S16LE};

/// The rate Windows is asked to synthesize at.
const WINDOWS_RATE: u32 = 22_050;

/// The slowest and fastest speech, as multiples of normal.
const SPEED: SpeedRange = SpeedRange::new(0.5, 2.0);

/// The words a recognizer's default pace is in, per minute, on macOS and in `espeak-ng`.
const WORDS_PER_MINUTE: f64 = 175.0;

const fn default_timeout() -> u32 {
    30
}

/// Lists the desktop voices, one per line: name, culture, gender.
const WINDOWS_LIST: &str = "Add-Type -AssemblyName System.Speech; \
$s = New-Object System.Speech.Synthesis.SpeechSynthesizer; \
foreach ($v in $s.GetInstalledVoices()) { if ($v.Enabled) { $i = $v.VoiceInfo; \
('{0}' + [char]9 + '{1}' + [char]9 + '{2}') -f $i.Name, $i.Culture.Name, $i.Gender } }";

/// Reads a line of text at a time from standard input and writes each as 16-bit mono PCM at 22050 Hz.
const WINDOWS_SAY: &str = "Add-Type -AssemblyName System.Speech; \
[Console]::InputEncoding = [System.Text.Encoding]::UTF8; \
$s = New-Object System.Speech.Synthesis.SpeechSynthesizer; \
if ($env:CANOA_SAY_VOICE) { $s.SelectVoice($env:CANOA_SAY_VOICE) }; \
if ($env:CANOA_SAY_RATE) { $s.Rate = [int]$env:CANOA_SAY_RATE }; \
$f = New-Object System.Speech.AudioFormat.SpeechAudioFormatInfo(22050, [System.Speech.AudioFormat.AudioBitsPerSample]::Sixteen, [System.Speech.AudioFormat.AudioChannel]::Mono); \
$o = [Console]::OpenStandardOutput(); \
while (($l = [Console]::In.ReadLine()) -ne $null) { if ($l.Trim().Length -eq 0) { continue }; \
$m = New-Object System.IO.MemoryStream; $s.SetOutputToAudioStream($m, $f); $s.Speak($l); $s.SetOutputToNull(); \
$b = $m.ToArray(); $o.Write($b, 0, $b.Length); $o.Flush() }";

/// `say` cannot write to a pipe, so it writes a file that is printed and removed.
const MACOS_SAY: &str = "f=$(mktemp -t canoa) || exit 1; \
say ${CANOA_SAY_VOICE:+-v \"$CANOA_SAY_VOICE\"} ${CANOA_SAY_RATE:+-r \"$CANOA_SAY_RATE\"} -o \"$f\" --file-format=WAVE --data-format=LEI16@22050 -f -; \
code=$?; cat \"$f\"; rm -f \"$f\"; exit $code";

/// What the system provider is configured with.
#[derive(ToValue, FromValue, Schema, Form, Debug, Clone, PartialEq)]
#[form(section(id = "system", label = "System voices"))]
#[non_exhaustive]
pub struct SystemSettings {
    /// The program that speaks, when not the one the system gives: Windows PowerShell, `say` or `espeak-ng`.
    #[schema(advanced, section = "system", title = "Program")]
    #[form(placeholder = "the system's own")]
    pub program: Option<String>,
    /// How long the program may go without a sound before it is stopped, in seconds.
    #[map(default = default_timeout())]
    #[schema(advanced, section = "system", title = "Time to start")]
    #[form(unit = "s")]
    pub timeout_seconds: u32,
}

impl Default for SystemSettings {
    fn default() -> Self {
        Self {
            program: None,
            timeout_seconds: default_timeout(),
        }
    }
}

/// The system provider.
#[derive(guatiao::Provider)]
#[provider(SpeechOutput, id = "speech_say_system", config = SystemSettings, form = SystemSettings)]
pub struct SystemSpeech {
    settings: SystemSettings,
}

impl From<SystemSettings> for SystemSpeech {
    fn from(settings: SystemSettings) -> Self {
        Self { settings }
    }
}

/// Which family of programs the computer the spawner runs on has.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Family {
    Windows,
    Macos,
    Espeak,
}

impl SystemSpeech {
    fn family(spawner: &Object<dyn ProcessSpawner>) -> Result<Family, ProviderError> {
        let info = spawner.describe()?;
        Ok(match info.os.as_str() {
            "windows" => Family::Windows,
            "macos" => Family::Macos,
            "android" | "ios" => {
                return Err(unsupported(
                    "this system gives programs no speech synthesizer",
                ));
            }
            _ => Family::Espeak,
        })
    }

    fn program(&self, family: Family) -> String {
        self.settings
            .program
            .clone()
            .filter(|program| !program.trim().is_empty())
            .unwrap_or_else(|| {
                match family {
                    Family::Windows => "powershell.exe",
                    Family::Macos => "/bin/sh",
                    Family::Espeak => "espeak-ng",
                }
                .to_owned()
            })
    }

    fn wait(&self) -> Duration {
        Duration::from_secs(u64::from(self.settings.timeout_seconds.max(1)))
    }
}

/// The spec that runs the Windows script `script`.
fn powershell(program: &str, script: &str) -> SpawnSpec {
    SpawnSpec::new(program)
        .arg("-NoProfile")
        .arg("-NonInteractive")
        .arg("-ExecutionPolicy")
        .arg("Bypass")
        .arg("-Command")
        .arg(script)
}

impl SpeechOutput for SystemSpeech {
    fn describe(&self) -> Result<OutputInfo, ProviderError> {
        let mut info = OutputInfo::new("System voices", vec![PCM_S16LE.to_string()]);
        info.description = Some("The voices of the computer the host runs on".to_owned());
        info.speed = Some(SPEED);
        info.uses = ["voice", "speed"].map(ToString::to_string).to_vec();
        info.leaves = Leaves::No;
        info.words = Some(
            "The operating system's own speech synthesizer on the host's computer speaks it; the text is sent nowhere."
                .to_owned(),
        );
        Ok(info)
    }

    fn voices(
        &self,
        query: VoiceQuery,
        spawner: Object<dyn ProcessSpawner>,
    ) -> Result<VoiceList, ProviderError> {
        let family = Self::family(&spawner)?;
        let program = self.program(family);
        let voices = match family {
            Family::Windows => parse_windows(&spoken::collect(
                &spawner,
                powershell(&program, WINDOWS_LIST),
                1 << 20,
                self.wait().max(Duration::from_secs(20)),
            )?),
            Family::Macos => {
                let spec = SpawnSpec::new("say").arg("-v").arg("?");
                parse_macos(&spoken::collect(&spawner, spec, 1 << 20, self.wait())?)
            }
            Family::Espeak => {
                let spec = SpawnSpec::new(&program).arg("--voices");
                parse_espeak(&spoken::collect(&spawner, spec, 1 << 20, self.wait())?)
            }
        };
        Ok(VoiceList::new(voices).speaking(query.language.as_deref()))
    }

    fn say(
        &self,
        request: SayRequest,
        spawner: Object<dyn ProcessSpawner>,
    ) -> Result<Object<dyn SpeechUtterance>, ProviderError> {
        let family = Self::family(&spawner)?;
        let text = checked_text(&request, None)?;
        let speed = request.speed.map(|speed| SPEED.clamp(speed));
        let program = self.program(family);
        let mut environment = Environment::new();
        if let Some(voice) = request.voice.as_deref().filter(|voice| !voice.is_empty()) {
            environment.insert("CANOA_SAY_VOICE", voice);
        }
        let (spec, lines, head, format) = match family {
            Family::Windows => {
                if let Some(speed) = speed {
                    // The synthesizer's rate is a step from -10 to 10; a step is about a tenth of normal speed.
                    #[allow(clippy::cast_possible_truncation, reason = "held to -10 and 10 first")]
                    let step = ((speed - 1.0) * 10.0).round().clamp(-10.0, 10.0) as i32;
                    environment.insert("CANOA_SAY_RATE", step.to_string());
                }
                (
                    powershell(&program, WINDOWS_SAY),
                    Some(sentences(text)),
                    Head::Raw,
                    AudioFormat::pcm(WINDOWS_RATE),
                )
            }
            Family::Macos => {
                if let Some(speed) = speed {
                    environment.insert("CANOA_SAY_RATE", per_minute(speed));
                }
                (
                    SpawnSpec::new(&program).arg("-c").arg(MACOS_SAY),
                    Some(vec![text.to_owned()]),
                    Head::Wav,
                    AudioFormat::pcm(22_050),
                )
            }
            Family::Espeak => {
                let mut spec = SpawnSpec::new(&program).arg("--stdout");
                if let Some(voice) = request.voice.as_deref().filter(|voice| !voice.is_empty()) {
                    spec = spec.arg("-v").arg(voice);
                }
                if let Some(speed) = speed {
                    spec = spec.arg("-s").arg(per_minute(speed));
                }
                (
                    spec,
                    Some(vec![text.to_owned()]),
                    Head::Wav,
                    AudioFormat::pcm(22_050),
                )
            }
        };
        let spec = spec.environment(environment);
        Ok(Spoken::start(&spawner, spec, lines, format, head, self.wait())?.into_object())
    }
}

/// Words a minute for `speed`.
fn per_minute(speed: f64) -> String {
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "held to 0.5 to 2 times 175"
    )]
    let wpm = (WORDS_PER_MINUTE * speed).round() as u32;
    wpm.to_string()
}

/// A voice of `culture` (`en-US`), as a language tag.
fn voice_in(id: &str, name: &str, culture: &str, gender: Option<&str>) -> Voice {
    let mut voice = Voice::new(id, name);
    let tag = culture.trim().replace('_', "-");
    if !tag.is_empty() {
        voice.language = Some(tag);
    }
    voice.gender = gender
        .map(|gender| gender.trim().to_ascii_lowercase())
        .filter(|gender| matches!(gender.as_str(), "female" | "male" | "neutral"));
    voice
}

/// The voices of the Windows listing: `name`, `culture` and `gender` between tabs, a voice a line.
fn parse_windows(listing: &str) -> Vec<Voice> {
    listing
        .lines()
        .filter_map(|line| {
            let mut parts = line.trim_end_matches('\r').split('\t');
            let name = parts.next()?.trim();
            (!name.is_empty())
                .then(|| voice_in(name, name, parts.next().unwrap_or(""), parts.next()))
        })
        .collect()
}

/// The voices of `say -v ?`: a name that may hold spaces, a locale, and after a `#` an example sentence.
fn parse_macos(listing: &str) -> Vec<Voice> {
    listing
        .lines()
        .filter_map(|line| {
            let head = line.split('#').next()?.trim_end();
            let (name, locale) = head.rsplit_once(char::is_whitespace)?;
            let name = name.trim();
            (!name.is_empty()).then(|| voice_in(name, name, locale, None))
        })
        .collect()
}

/// The voices of `espeak-ng --voices`: columns `Pty Language Age/Gender VoiceName File Other`, the first line the headings.
fn parse_espeak(listing: &str) -> Vec<Voice> {
    listing
        .lines()
        .skip(1)
        .filter_map(|line| {
            let mut columns = line.split_whitespace();
            let _priority = columns.next()?;
            let language = columns.next()?;
            let age_gender = columns.next()?;
            let name = columns.next()?;
            let gender = match age_gender.rsplit('/').next() {
                Some("M") => Some("male"),
                Some("F") => Some("female"),
                _ => None,
            };
            Some(voice_in(
                language,
                &name.replace('_', " "),
                language,
                gender,
            ))
        })
        .collect()
}

impl core::fmt::Debug for SystemSpeech {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("SystemSpeech").finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_windows_listing_is_a_voice_a_line() {
        let voices = parse_windows(
            "Microsoft David Desktop\ten-US\tMale\r\nMicrosoft Zira Desktop\ten-US\tFemale\r\n\r\n",
        );
        assert_eq!(voices.len(), 2);
        assert_eq!(voices[1].id, "Microsoft Zira Desktop");
        assert_eq!(voices[1].language.as_deref(), Some("en-US"));
        assert_eq!(voices[1].gender.as_deref(), Some("female"));
    }

    #[test]
    fn the_macos_listing_keeps_a_name_with_spaces() {
        let voices = parse_macos(
            "Alex                en_US    # Most people recognize me by my voice.\nEddy (English (US)) en_US    # Hello! My name is Eddy.\n",
        );
        assert_eq!(voices[0].id, "Alex");
        assert_eq!(voices[0].language.as_deref(), Some("en-US"));
        assert_eq!(voices[1].id, "Eddy (English (US))");
    }

    #[test]
    fn the_espeak_listing_skips_its_headings() {
        let voices = parse_espeak(
            "Pty Language       Age/Gender VoiceName          File                 Other Languages\n 5  af              --/M      Afrikaans          gmw/af\n 5  en-us           --/M      English_(America)  gmw/en-US           (en 3)\n",
        );
        assert_eq!(voices.len(), 2);
        assert_eq!(voices[1].id, "en-us");
        assert_eq!(voices[1].name, "English (America)");
        assert_eq!(voices[1].gender.as_deref(), Some("male"));
    }

    #[test]
    fn a_speed_is_words_a_minute() {
        assert_eq!(per_minute(1.0), "175");
        assert_eq!(per_minute(2.0), "350");
    }
}
