//! The models Candle runs: OpenAI's Whisper in the `safetensors` form of the publisher's Hugging Face repositories, each at one pinned
//! revision and each file checked against the SHA-256 that revision lists. The models are MIT licensed. They are installed on their
//! own from the Models page ([`crate::WhisperCandleModels`]); nothing else is installed, since the speech runs inside the host.

/// Where a repository is downloaded from.
const HUB: &str = "https://huggingface.co/openai/";

/// Where the models are downloaded from: the publisher's repositories, and the hosts Hugging Face sends a download on to.
pub const ENDPOINTS: [&str; 6] = [
    HUB,
    "https://cdn-lfs.hf.co",
    "https://cdn-lfs-us-1.hf.co",
    "https://cdn-lfs-eu-1.hf.co",
    "https://cas-bridge.xethub.hf.co",
    "https://us.aws.cdn.hf.co",
];

/// The names of a model's files, which the folder `<id>/` of the models folder holds.
pub const CONFIG: &str = "config.json";
/// The tokenizer's file.
pub const TOKENIZER: &str = "tokenizer.json";
/// The weights' file.
pub const WEIGHTS: &str = "model.safetensors";
/// The three together.
pub const FILE_NAMES: [&str; 3] = [CONFIG, TOKENIZER, WEIGHTS];

/// One file of a model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct File {
    /// Its name in the repository and in the model's folder.
    pub name: &'static str,
    /// Its size in bytes.
    pub bytes: u64,
    /// Its SHA-256, as the publisher's listing gives it.
    pub sha256: &'static str,
}

/// One model of the publisher's list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Model {
    /// What the page and `speech/offers` call it, and the name of its folder.
    pub id: &'static str,
    /// What a person reads: what it is good for and how big it is.
    pub label: &'static str,
    /// The publisher's repository, `openai/<repository>`.
    pub repository: &'static str,
    /// The revision of the repository the digests were read from.
    pub revision: &'static str,
    /// Its files: the configuration, the tokenizer and the weights.
    pub files: [File; 3],
}

impl Model {
    /// The address of one of its files.
    #[must_use]
    pub fn url(&self, file: &File) -> String {
        format!(
            "{HUB}{}/resolve/{}/{}",
            self.repository, self.revision, file.name
        )
    }

    /// Its size on disk: the three files together.
    #[must_use]
    pub fn bytes(&self) -> u64 {
        self.files.iter().map(|file| file.bytes).sum()
    }

    /// The file `name` of the three.
    #[must_use]
    pub fn file(&self, name: &str) -> Option<&File> {
        self.files.iter().find(|file| file.name == name)
    }
}

/// The models offered, the default first. All of them take every language Whisper does; the weights are 16-bit floats, which Candle
/// reads as 32-bit ones, so a model takes about twice its size in memory.
pub const MODELS: [Model; 4] = [
    Model {
        id: "base",
        label: "Base, every language (291 MB)",
        repository: "whisper-base",
        revision: "e37978b90ca9030d5170a5c07aadb050351a65bb",
        files: [
            File {
                name: CONFIG,
                bytes: 1_983,
                sha256: "a153c53883a6799b6f056b4a8d1a515c9926d03994682ba88a7616618d7da0c1",
            },
            File {
                name: TOKENIZER,
                bytes: 2_480_466,
                sha256: "27fc476bfe7f17299480be2273fc0608e4d5a99aba2ab5dec5374b4482d1a566",
            },
            File {
                name: WEIGHTS,
                bytes: 290_403_936,
                sha256: "07cadb9f25677c8d50df603e66a98fbd842cce45047139baeb16e6219a1e807b",
            },
        ],
    },
    Model {
        id: "tiny",
        label: "Tiny, every language (154 MB)",
        repository: "whisper-tiny",
        revision: "169d4a4341b33bc18d8881c4b69c2e104e1cc0af",
        files: [
            File {
                name: CONFIG,
                bytes: 1_983,
                sha256: "ffdccec4f3211f4c63310f2b7098f309fe70f3952cedc5e4d11e43f5b2379b98",
            },
            File {
                name: TOKENIZER,
                bytes: 2_480_466,
                sha256: "27fc476bfe7f17299480be2273fc0608e4d5a99aba2ab5dec5374b4482d1a566",
            },
            File {
                name: WEIGHTS,
                bytes: 151_061_672,
                sha256: "7ebd0e69e78190ffe1438491fa05cc1f5c1aa3a4c4db3bc1723adbb551ea2395",
            },
        ],
    },
    Model {
        id: "small",
        label: "Small, every language (970 MB)",
        repository: "whisper-small",
        revision: "973afd24965f72e36ca33b3055d56a652f456b4d",
        files: [
            File {
                name: CONFIG,
                bytes: 1_967,
                sha256: "e6a2b489da1b5aed65a8eb8d1e7466fa867ad5643a8bc138ba708bd56b2875c4",
            },
            File {
                name: TOKENIZER,
                bytes: 2_480_466,
                sha256: "27fc476bfe7f17299480be2273fc0608e4d5a99aba2ab5dec5374b4482d1a566",
            },
            File {
                name: WEIGHTS,
                bytes: 966_995_080,
                sha256: "1d7734884874f1a1513ed9aa760a4f8e97aaa02fd6d93a3a85d27b2ae9ca596b",
            },
        ],
    },
    Model {
        id: "large-v3-turbo",
        label: "Large v3 turbo, every language (1.6 GB)",
        repository: "whisper-large-v3-turbo",
        revision: "41f01f3fe87f28c78e2fbf8b568835947dd65ed9",
        files: [
            File {
                name: CONFIG,
                bytes: 1_256,
                sha256: "c5b526b3e3cd64cd8940dabb45e8ba726629e22d8ed389c29b552f9140daf04a",
            },
            File {
                name: TOKENIZER,
                bytes: 2_710_337,
                sha256: "297b13372ac43916285644fb9687add3cc62ee2a1adb60da3dc25cc94c1871fd",
            },
            File {
                name: WEIGHTS,
                bytes: 1_617_824_864,
                sha256: "542566a422ae4f3fd23f1ba11add198fca01bbf82e66e6a2857b3f608b1eb9d1",
            },
        ],
    },
];

/// The model with the id `id`.
#[must_use]
pub fn model(id: &str) -> Option<&'static Model> {
    MODELS.iter().find(|model| model.id == id)
}
