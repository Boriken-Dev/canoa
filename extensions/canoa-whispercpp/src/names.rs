//! What a Hugging Face Whisper folder calls things, and what whisper.cpp calls them.

use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use std::collections::HashMap;

/// The tensors a GGML file holds, in the order of OpenAI's state dict, which is the order of the files whisper.cpp publishes.
pub(crate) fn tensor_order(encoder_layers: usize, decoder_layers: usize) -> Vec<String> {
    let mut order: Vec<String> = [
        "decoder.positional_embedding",
        "encoder.positional_embedding",
        "decoder.token_embedding.weight",
    ]
    .map(String::from)
    .to_vec();
    order.extend(blocks("decoder", decoder_layers, true));
    order.extend(["decoder.ln.weight", "decoder.ln.bias"].map(String::from));
    order.extend(
        [
            "encoder.conv1.weight",
            "encoder.conv1.bias",
            "encoder.conv2.weight",
            "encoder.conv2.bias",
        ]
        .map(String::from),
    );
    order.extend(blocks("encoder", encoder_layers, false));
    order.extend(["encoder.ln_post.weight", "encoder.ln_post.bias"].map(String::from));
    order
}

fn blocks(side: &str, layers: usize, cross: bool) -> Vec<String> {
    let mut names = Vec::new();
    for layer in 0..layers {
        let block = format!("{side}.blocks.{layer}");
        for part in [
            "mlp_ln.weight",
            "mlp_ln.bias",
            "mlp.0.weight",
            "mlp.0.bias",
            "mlp.2.weight",
            "mlp.2.bias",
            "attn_ln.weight",
            "attn_ln.bias",
        ] {
            names.push(format!("{block}.{part}"));
        }
        let mut attentions = vec![("attn", false)];
        if cross {
            attentions.push(("cross_attn_ln", true));
            attentions.push(("cross_attn", false));
        }
        for (attention, is_norm) in attentions {
            if is_norm {
                names.push(format!("{block}.{attention}.weight"));
                names.push(format!("{block}.{attention}.bias"));
                continue;
            }
            for part in [
                "query.weight",
                "query.bias",
                "key.weight",
                "value.weight",
                "value.bias",
                "out.weight",
                "out.bias",
            ] {
                names.push(format!("{block}.{attention}.{part}"));
            }
        }
    }
    names
}

/// whisper.cpp's name for a Hugging Face tensor name, or `None` for a layer tensor whisper.cpp has no tensor for. A name that is not in a layer is returned as it is.
pub(crate) fn ggml_name(hf: &str) -> Option<String> {
    let whole = |name: &str| -> Option<&'static str> {
        Some(match name {
            "encoder.layer_norm.bias" => "encoder.ln_post.bias",
            "encoder.layer_norm.weight" => "encoder.ln_post.weight",
            "encoder.embed_positions.weight" => "encoder.positional_embedding",
            "decoder.layer_norm.bias" => "decoder.ln.bias",
            "decoder.layer_norm.weight" => "decoder.ln.weight",
            "decoder.embed_positions.weight" => "decoder.positional_embedding",
            "decoder.embed_tokens.weight" => "decoder.token_embedding.weight",
            _ => return None,
        })
    };
    let layer = |name: &str| -> Option<&'static str> {
        Some(match name {
            "self_attn.k_proj" => "attn.key",
            "self_attn.q_proj" => "attn.query",
            "self_attn.v_proj" => "attn.value",
            "self_attn.out_proj" => "attn.out",
            "self_attn_layer_norm" => "attn_ln",
            "encoder_attn.k_proj" => "cross_attn.key",
            "encoder_attn.q_proj" => "cross_attn.query",
            "encoder_attn.v_proj" => "cross_attn.value",
            "encoder_attn.out_proj" => "cross_attn.out",
            "encoder_attn_layer_norm" => "cross_attn_ln",
            "fc1" => "mlp.0",
            "fc2" => "mlp.2",
            "final_layer_norm" => "mlp_ln",
            _ => return None,
        })
    };
    let rest = hf.strip_prefix("model.").unwrap_or(hf);
    let parts: Vec<&str> = rest.split('.').collect();
    if parts.len() > 4 && parts[1] == "layers" && matches!(parts[0], "encoder" | "decoder") {
        let mapped = layer(&parts[3..parts.len() - 1].join("."))?;
        return Some(format!(
            "{}.blocks.{}.{mapped}.{}",
            parts[0],
            parts[2],
            parts[parts.len() - 1]
        ));
    }
    Some(whole(rest).map_or_else(|| String::from(rest), String::from))
}

/// GPT-2's table from the characters a byte-level BPE vocabulary is written in back to the bytes they stand for.
pub(crate) fn byte_decoder() -> HashMap<char, u8> {
    let mut bytes: Vec<u32> = (u32::from(b'!')..=u32::from(b'~'))
        .chain(0xA1..=0xAC)
        .chain(0xAE..=0xFF)
        .collect();
    let mut chars = bytes.clone();
    let mut extra = 0;
    for byte in 0..256u32 {
        if !bytes.contains(&byte) {
            bytes.push(byte);
            chars.push(256 + extra);
            extra += 1;
        }
    }
    bytes
        .iter()
        .zip(chars)
        .filter_map(|(&byte, ch)| Some((char::from_u32(ch)?, u8::try_from(byte).ok()?)))
        .collect()
}
