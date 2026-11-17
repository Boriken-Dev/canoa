//! The command provider for speech output: a program the person names that is handed text and writes audio.
//!
//! The program is started, through the spawner the host hands the kind, when an utterance starts and ends when it ends. The
//! text is on its standard input a sentence a line (`input = "stdin"`) or in an argument named `{text}`
//! (`input = "argument"`); what it writes to standard output is the audio, as raw 16-bit samples at the setting's `rate` or as
//! a WAV stream.
//!
//! The voices, models and styles are the lists the person writes in the settings: the program is not asked for them.

use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;
use core::time::Duration;

use coqui_acp_api::{Environment, ProcessSpawner, SpawnSpec};
use guatiao::library::{Object, ProviderError};
use guatiao::{FromValue, Schema, ToValue};
use guatiao_intake::Form;

use super::spoken::{Head, Spoken};
use crate::speech::output::{
    Choice, Leaves, OutputInfo, SayRequest, SpeechOutput, SpeechUtterance, SpeedRange, Voice,
    VoiceList, VoiceQuery, checked_text, sentences, unknown_choice,
};
use crate::speech::{AudioFormat, PCM_S16LE, bad_value};

/// The slowest and fastest speech, as multiples of normal, for a program that takes a speed.
const SPEED: SpeedRange = SpeedRange::new(0.5, 2.0);

/// Where the text goes.
#[derive(ToValue, FromValue, Schema, Debug, Clone, Copy, Default, PartialEq, Eq)]
#[map(rename_all = "snake_case")]
pub enum SayInput {
    /// The program's standard input, a sentence a line.
    #[default]
    Stdin,
    /// The argument that names `{text}`, the whole text in one.
    Argument,
}

/// What the program writes.
#[derive(ToValue, FromValue, Schema, Debug, Clone, Copy, Default, PartialEq, Eq)]
#[map(rename_all = "snake_case")]
pub enum SayOutput {
    /// Raw 16-bit little-endian mono samples at the setting's `rate`.
    #[default]
    Pcm,
    /// A WAV stream of 16-bit mono samples, read from its `data` chunk on; its header's rate is not read, so `rate` says it.
    Wav,
}

const fn default_input() -> SayInput {
    SayInput::Stdin
}

const fn default_output() -> SayOutput {
    SayOutput::Pcm
}

const fn default_rate() -> u32 {
    22_050
}

const fn default_timeout() -> u32 {
    60
}

/// What the command provider is configured with.
///
/// Each of `args` may name `{voice}`, `{model}`, `{style}`, `{speed}`, `{language}` and `{text}`, which are replaced with what
/// the request chose (a choice left out is the empty text); the same choices are set as the variables `CANOA_SAY_VOICE`,
/// `CANOA_SAY_MODEL`, `CANOA_SAY_STYLE`, `CANOA_SAY_SPEED` and `CANOA_SAY_LANGUAGE`.
#[derive(ToValue, FromValue, Schema, Form, Debug, Clone, PartialEq)]
#[form(section(id = "command", label = "Command"))]
#[non_exhaustive]
pub struct CommandSaySettings {
    /// The program to run: a name found on the search path, or a path.
    #[schema(section = "command", title = "Program")]
    #[form(placeholder = "my-tts")]
    pub program: String,
    /// Its arguments, one per entry.
    #[map(default)]
    #[schema(section = "command", title = "Arguments")]
    pub args: Vec<String>,
    /// Where it wants the text.
    #[map(default = default_input())]
    #[schema(section = "command", title = "Text")]
    pub input: SayInput,
    /// What it writes.
    #[map(default = default_output())]
    #[schema(section = "command", title = "Audio on standard output")]
    pub output: SayOutput,
    /// The samples a second of the audio it writes.
    #[map(default = default_rate())]
    #[schema(section = "command", title = "Sample rate")]
    #[form(unit = "Hz")]
    pub rate: u32,
    /// The voices it has, as the names it takes.
    #[map(default)]
    #[schema(section = "command", title = "Voices")]
    pub voices: Vec<String>,
    /// The models it has.
    #[map(default)]
    #[schema(section = "command", title = "Models")]
    pub models: Vec<String>,
    /// The styles it has.
    #[map(default)]
    #[schema(section = "command", title = "Styles")]
    pub styles: Vec<String>,
    /// Whether it takes a speed, as `{speed}` or `CANOA_SAY_SPEED`.
    #[map(default)]
    #[schema(section = "command", title = "Takes a speed")]
    pub speed: bool,
    /// How long it may go without a sound before it is stopped, in seconds.
    #[map(default = default_timeout())]
    #[schema(advanced, section = "command", title = "Time to start")]
    #[form(unit = "s")]
    pub timeout_seconds: u32,
}

impl CommandSaySettings {
    /// Settings that run `program` with no arguments, the text on its input and raw samples at 22050 Hz out.
    #[must_use]
    pub fn new(program: impl Into<String>) -> Self {
        Self {
            program: program.into(),
            args: Vec::new(),
            input: default_input(),
            output: default_output(),
            rate: default_rate(),
            voices: Vec::new(),
            models: Vec::new(),
            styles: Vec::new(),
            speed: false,
            timeout_seconds: default_timeout(),
        }
    }
}

/// The command provider for speech output.
#[derive(guatiao::Provider)]
#[provider(SpeechOutput, id = "speech_say_command", config = CommandSaySettings, form = CommandSaySettings)]
pub struct CommandSay {
    settings: CommandSaySettings,
}

impl From<CommandSaySettings> for CommandSay {
    fn from(settings: CommandSaySettings) -> Self {
        Self { settings }
    }
}

fn choices(names: &[String]) -> Vec<Choice> {
    names
        .iter()
        .filter(|name| !name.trim().is_empty())
        .map(|name| Choice::new(name.trim(), name.trim()))
        .collect()
}

/// `wanted`, when the provider lists any and it is one of them.
fn listed<'a>(
    what: &str,
    wanted: Option<&'a str>,
    has: &[String],
) -> Result<Option<&'a str>, ProviderError> {
    let has: Vec<String> = has
        .iter()
        .map(|name| name.trim().to_owned())
        .filter(|name| !name.is_empty())
        .collect();
    match wanted.filter(|wanted| !wanted.is_empty()) {
        Some(wanted) if !has.is_empty() && !has.iter().any(|name| name == wanted) => {
            Err(unknown_choice(what, wanted, &has))
        }
        other => Ok(other),
    }
}

impl SpeechOutput for CommandSay {
    fn describe(&self) -> Result<OutputInfo, ProviderError> {
        let settings = &self.settings;
        let mut info = OutputInfo::new("Command", vec![PCM_S16LE.to_string()]);
        info.description = Some("Runs a program that reads text and writes audio".to_owned());
        info.models = choices(&settings.models);
        info.default_model = info.models.first().map(|choice| choice.id.clone());
        info.styles = choices(&settings.styles);
        info.speed = settings.speed.then_some(SPEED);
        info.uses = [
            (!settings.voices.is_empty()).then_some("voice"),
            (!settings.models.is_empty()).then_some("model"),
            (!settings.styles.is_empty()).then_some("style"),
            settings.speed.then_some("speed"),
            Some("language"),
        ]
        .into_iter()
        .flatten()
        .map(ToString::to_string)
        .collect();
        info.leaves = Leaves::Maybe;
        info.words = Some(
            "The text goes to the program you named on the host's computer, which may send it further; Canoa does not."
                .to_owned(),
        );
        Ok(info)
    }

    fn voices(
        &self,
        query: VoiceQuery,
        _spawner: Object<dyn ProcessSpawner>,
    ) -> Result<VoiceList, ProviderError> {
        let voices = self
            .settings
            .voices
            .iter()
            .filter(|name| !name.trim().is_empty())
            .map(|name| Voice::new(name.trim(), name.trim()))
            .collect();
        Ok(VoiceList::new(voices).speaking(query.language.as_deref()))
    }

    fn say(
        &self,
        request: SayRequest,
        spawner: Object<dyn ProcessSpawner>,
    ) -> Result<Object<dyn SpeechUtterance>, ProviderError> {
        let settings = &self.settings;
        if settings.program.trim().is_empty() {
            return Err(bad_value("the command to run is not set"));
        }
        let text = checked_text(&request, None)?;
        let voice = listed("voice", request.voice.as_deref(), &settings.voices)?.unwrap_or("");
        let model = listed("model", request.model.as_deref(), &settings.models)?
            .or_else(|| settings.models.first().map(String::as_str))
            .unwrap_or("");
        let style = listed("style", request.style.as_deref(), &settings.styles)?.unwrap_or("");
        let speed = request
            .speed
            .filter(|_| settings.speed)
            .map(|speed| alloc::format!("{:.2}", SPEED.clamp(speed)))
            .unwrap_or_default();
        let language = request.language.as_deref().unwrap_or("");
        let mut spec = SpawnSpec::new(&settings.program);
        for arg in &settings.args {
            spec = spec.arg(
                arg.replace("{voice}", voice)
                    .replace("{model}", model)
                    .replace("{style}", style)
                    .replace("{speed}", &speed)
                    .replace("{language}", language)
                    .replace("{text}", text),
            );
        }
        let mut environment = Environment::new();
        for (name, value) in [
            ("VOICE", voice),
            ("MODEL", model),
            ("STYLE", style),
            ("SPEED", speed.as_str()),
            ("LANGUAGE", language),
        ] {
            if !value.is_empty() {
                environment.insert(alloc::format!("CANOA_SAY_{name}"), value);
            }
        }
        spec = spec.environment(environment);
        let lines = match settings.input {
            SayInput::Stdin => Some(sentences(text)),
            SayInput::Argument => None,
        };
        let head = match settings.output {
            SayOutput::Pcm => Head::Raw,
            SayOutput::Wav => Head::Wav,
        };
        let idle = Duration::from_secs(u64::from(settings.timeout_seconds.max(1)));
        Ok(Spoken::start(
            &spawner,
            spec,
            lines,
            AudioFormat::pcm(settings.rate.max(1)),
            head,
            idle,
        )?
        .into_object())
    }
}

impl core::fmt::Debug for CommandSay {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("CommandSay")
            .field("program", &self.settings.program)
            .finish_non_exhaustive()
    }
}
