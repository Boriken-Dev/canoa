//! The models whisper.cpp runs: the `ggml-*.bin` files of the publisher's Hugging Face repository at one pinned revision, each
//! checked against the SHA-256 that revision's listing gives. The models are MIT licensed. They are installed on their own from the
//! Models page ([`crate::WhisperModels`]); nothing else is installed, since the speech runs inside the host.

/// The models' folder at the revision the digests below were read from.
pub const MODELS_ORIGIN: &str = "https://huggingface.co/ggerganov/whisper.cpp/resolve/5359861c739e955e79d9a303bcbc70fb988958b1/";

/// Where the models are downloaded from: the publisher's repository, and the hosts Hugging Face sends a download on to.
pub const ENDPOINTS: [&str; 6] = [
    MODELS_ORIGIN,
    "https://cdn-lfs.hf.co",
    "https://cdn-lfs-us-1.hf.co",
    "https://cdn-lfs-eu-1.hf.co",
    "https://cas-bridge.xethub.hf.co",
    "https://us.aws.cdn.hf.co",
];

/// One model of the publisher's list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Model {
    /// What the page and `speech/offers` call it.
    pub id: &'static str,
    /// What a person reads: what it is good for and how big it is.
    pub label: &'static str,
    /// The file in [`MODELS_ORIGIN`].
    pub file: &'static str,
    /// Its size in bytes.
    pub bytes: u64,
    /// Its SHA-256, as the publisher's listing gives it.
    pub sha256: &'static str,
}

/// The models offered, the default first. Multilingual ones take Spanish and English alike; the `.en` ones are English only and
/// a little better at it for their size.
pub const MODELS: [Model; 8] = [
    Model {
        id: "base",
        label: "Base, every language (148 MB)",
        file: "ggml-base.bin",
        bytes: 147_951_465,
        sha256: "60ed5bc3dd14eea856493d334349b405782ddcaf0028d4b5df4088345fba2efe",
    },
    Model {
        id: "base.en",
        label: "Base, English only (148 MB)",
        file: "ggml-base.en.bin",
        bytes: 147_964_211,
        sha256: "a03779c86df3323075f5e796cb2ce5029f00ec8869eee3fdfb897afe36c6d002",
    },
    Model {
        id: "tiny",
        label: "Tiny, every language (78 MB)",
        file: "ggml-tiny.bin",
        bytes: 77_691_713,
        sha256: "be07e048e1e599ad46341c8d2a135645097a538221678b7acdd1b1919c6e1b21",
    },
    Model {
        id: "tiny.en",
        label: "Tiny, English only (78 MB)",
        file: "ggml-tiny.en.bin",
        bytes: 77_704_715,
        sha256: "921e4cf8686fdd993dcd081a5da5b6c365bfde1162e72b08d75ac75289920b1f",
    },
    Model {
        id: "small-q5_1",
        label: "Small, every language, compressed (190 MB)",
        file: "ggml-small-q5_1.bin",
        bytes: 190_085_487,
        sha256: "ae85e4a935d7a567bd102fe55afc16bb595bdb618e11b2fc7591bc08120411bb",
    },
    Model {
        id: "small",
        label: "Small, every language (488 MB)",
        file: "ggml-small.bin",
        bytes: 487_601_967,
        sha256: "1be3a9b2063867b937e64e2ec7483364a79917e157fa98c5d94b5c1fffea987b",
    },
    Model {
        id: "small.en",
        label: "Small, English only (488 MB)",
        file: "ggml-small.en.bin",
        bytes: 487_614_201,
        sha256: "c6138d6d58ecc8322097e0f987c32f1be8bb0a18532a3f88f734d1bbf9c41e5d",
    },
    Model {
        id: "large-v3-turbo-q5_0",
        label: "Large v3 turbo, every language, compressed (574 MB)",
        file: "ggml-large-v3-turbo-q5_0.bin",
        bytes: 574_041_195,
        sha256: "394221709cd5ad1f40c46e6031ca61bce88931e6e088c188294c6d5a55ffa7e2",
    },
];

/// The model with the id `id`.
#[must_use]
pub fn model(id: &str) -> Option<&'static Model> {
    MODELS.iter().find(|model| model.id == id)
}
