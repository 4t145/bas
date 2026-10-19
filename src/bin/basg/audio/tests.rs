use super::*;
use rodio::Source;

fn wav() -> acp::AudioContent {
    let mut bytes = Vec::new();
    bytes.extend(b"RIFF");
    bytes.extend(44u32.to_le_bytes());
    bytes.extend(b"WAVEfmt ");
    bytes.extend(16u32.to_le_bytes());
    bytes.extend(1u16.to_le_bytes()); // PCM
    bytes.extend(1u16.to_le_bytes()); // mono
    bytes.extend(8_000u32.to_le_bytes());
    bytes.extend(16_000u32.to_le_bytes());
    bytes.extend(2u16.to_le_bytes());
    bytes.extend(16u16.to_le_bytes());
    bytes.extend(b"data");
    bytes.extend(8u32.to_le_bytes());
    bytes.extend([0u8; 8]);
    acp::AudioContent::new(
        base64::engine::general_purpose::STANDARD.encode(bytes),
        "audio/wav",
    )
}

#[test]
fn decodes_native_audio_without_opening_a_device() {
    let decoded = decode(&wav()).expect("WAV");
    assert_eq!(decoded.sample_rate(), 8_000);
    assert_eq!(decoded.channels(), 1);
    assert_eq!(decoded.collect::<Vec<_>>(), vec![0.; 4]);
}

#[test]
fn malformed_data_and_unsupported_files_are_errors() {
    assert!(matches!(
        decode(&acp::AudioContent::new("!!!", "audio/wav")),
        Err(Error::Base64(_))
    ));
    assert!(matches!(
        decode(&acp::AudioContent::new(
            base64::engine::general_purpose::STANDARD.encode(b"not audio"),
            "audio/wav",
        )),
        Err(Error::Decoder(_))
    ));
}
