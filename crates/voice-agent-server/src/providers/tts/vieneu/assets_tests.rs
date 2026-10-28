use super::*;
use std::collections::HashSet;

#[test]
fn all_preset_voice_ids_and_source_names_are_unique() {
    let turbo_ids: HashSet<_> = TURBO_VOICES.iter().map(|voice| voice.id).collect();
    let turbo_names: HashSet<_> = TURBO_VOICES.iter().map(|voice| voice.source_name).collect();
    assert_eq!(TURBO_VOICES.len(), 25);
    assert_eq!(turbo_ids.len(), TURBO_VOICES.len());
    assert_eq!(turbo_names.len(), TURBO_VOICES.len());
    assert!(turbo_ids.contains(DEFAULT_VOICE));

    let nano_ids: HashSet<_> = NANO_VOICES.iter().map(|voice| voice.id).collect();
    let nano_names: HashSet<_> = NANO_VOICES.iter().map(|voice| voice.source_name).collect();
    assert_eq!(NANO_VOICES.len(), 11);
    assert_eq!(nano_ids.len(), NANO_VOICES.len());
    assert_eq!(nano_names.len(), NANO_VOICES.len());
    assert!(nano_ids.contains(NANO_DEFAULT_VOICE));
}

#[test]
fn every_asset_source_is_revision_pinned() {
    assert_eq!(CORE_ASSETS.len(), 13);
    assert_eq!(NANO_ASSETS.len(), 8);
    assert_eq!(ALL_ASSETS.len(), 20);
    let paths: HashSet<_> = ALL_ASSETS.iter().map(|asset| asset.path).collect();
    assert_eq!(paths.len(), ALL_ASSETS.len());
    for asset in ALL_ASSETS {
        assert!(
            [
                MODEL_REVISION,
                CODEC_REVISION,
                PRESET_REVISION,
                NANO_MODEL_REVISION,
                NANO_PRESET_REVISION,
                G2P_REVISION,
            ]
            .iter()
            .any(|revision| asset.url.contains(revision)),
            "unpinned asset: {}",
            asset.path
        );
        assert!(asset.url.starts_with("https://"));
        assert!(!asset.url.contains("/resolve/main/"));
    }
}
