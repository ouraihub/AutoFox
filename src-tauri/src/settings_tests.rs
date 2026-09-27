//! Settings tests: download directory validation rules.

use super::validate_download_directory;
use crate::ipc::ErrorCode;

/// The directory a user picks is usually absolute and already exists. Both of
/// those are accepted, and so is an absolute path that does not exist yet:
/// `download.rs` runs `create_dir_all` before writing, so rejecting it here
/// would block the legitimate "put downloads in a new folder" case.
#[test]
fn accepts_absolute_paths_whether_or_not_they_exist() {
    let existing = std::env::temp_dir();
    validate_download_directory(&existing.to_string_lossy()).expect("existing dir accepted");

    let missing = existing.join(format!("autofox-settings-{}", uuid::Uuid::new_v4()));
    assert!(!missing.exists(), "fresh uuid path must not exist");
    validate_download_directory(&missing.to_string_lossy()).expect("missing dir accepted");
}

/// A relative path resolves against the process working directory, which for a
/// packaged app is wherever the shell happened to launch it from. Accepting one
/// would scatter downloads unpredictably, so it is a validation error on the
/// `downloadDirectory` field rather than a silent normalisation.
#[test]
fn rejects_relative_and_blank_paths() {
    for input in ["downloads", "./downloads", "", "   "] {
        let error = validate_download_directory(input)
            .expect_err("relative or blank path must be rejected");
        assert_eq!(error.code, ErrorCode::Validation);
        assert_eq!(error.field, Some("downloadDirectory"));
        assert_eq!(error.message_key, "errors.invalidDownloadDirectory");
    }
}

/// An existing *file* is the one case where `create_dir_all` fails later with
/// an opaque OS error, so it is worth catching at save time where the message
/// can name the offending field.
#[test]
fn rejects_a_path_that_is_a_file() {
    let file = std::env::temp_dir().join(format!("autofox-settings-{}.txt", uuid::Uuid::new_v4()));
    std::fs::write(&file, b"not a directory").expect("write probe file");

    let error = validate_download_directory(&file.to_string_lossy())
        .expect_err("a file path must be rejected");
    assert_eq!(error.field, Some("downloadDirectory"));

    std::fs::remove_file(&file).expect("remove probe file");
}
