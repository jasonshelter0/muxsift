use std::path::{Path, PathBuf};

pub fn sanitize_filename(value: &str) -> String {
    let cleaned: String = value
        .chars()
        .map(|character| {
            if character.is_control()
                || matches!(
                    character,
                    '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*'
                )
            {
                '_'
            } else {
                character
            }
        })
        .collect();
    let cleaned = cleaned.trim().trim_end_matches(['.', ' ']);
    let reserved = [
        "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
        "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
    ];
    let mut result = if cleaned.is_empty() {
        "untitled".to_owned()
    } else {
        cleaned.to_owned()
    };
    if reserved
        .iter()
        .any(|name| result.eq_ignore_ascii_case(name))
    {
        result.insert(0, '_');
    }
    while result.len() > 180 {
        result.pop();
    }
    result
}

pub fn final_output_path(directory: &Path, title: &str) -> PathBuf {
    final_output_path_with_extension(directory, title, "mp4")
}

pub fn final_output_path_with_extension(directory: &Path, title: &str, extension: &str) -> PathBuf {
    directory.join(format!("{}.{}", sanitize_filename(title), extension))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handles_windows_names_and_characters() {
        assert_eq!(sanitize_filename("CON"), "_CON");
        assert_eq!(sanitize_filename(" a:b?. "), "a_b_");
        assert_eq!(sanitize_filename("中文 title"), "中文 title");
    }

    #[test]
    fn supports_audio_extension() {
        assert_eq!(
            final_output_path_with_extension(Path::new("out"), "title", "m4a"),
            PathBuf::from("out/title.m4a")
        );
    }
}
