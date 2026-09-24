//! Steam library discovery.

use std::path::PathBuf;

/// Steam app id of Guild Wars 2.
pub const GW2_APP_ID: u32 = 1_284_210;

/// Install directory of Guild Wars 2 relative to a Steam library folder.
pub const GW2_INSTALL_DIR: &str = r"steamapps\common\Guild Wars 2";

/// Extracts all library folder paths from the contents of `steamapps\libraryfolders.vdf`.
///
/// Only the `"path"` entries are of interest, so instead of a full VDF parser this scans the
/// quoted tokens and returns every value that follows a `"path"` key.
pub fn library_folders(vdf: &str) -> Vec<PathBuf> {
    let tokens = quoted_tokens(vdf);
    tokens
        .windows(2)
        .filter(|pair| pair[0].eq_ignore_ascii_case("path"))
        .map(|pair| PathBuf::from(&pair[1]))
        .collect()
}

/// Returns all double-quoted strings in `text`, with `\\` and `\"` escapes resolved.
fn quoted_tokens(text: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c != '"' {
            continue;
        }
        let mut token = String::new();
        while let Some(c) = chars.next() {
            match c {
                '"' => break,
                '\\' => {
                    if let Some(escaped) = chars.next() {
                        token.push(escaped);
                    }
                }
                _ => token.push(c),
            }
        }
        tokens.push(token);
    }
    tokens
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#""libraryfolders"
{
	"0"
	{
		"path"		"C:\\Program Files (x86)\\Steam"
		"label"		""
		"apps"
		{
			"228980"		"128606066"
		}
	}
	"1"
	{
		"path"		"D:\\SteamLibrary"
		"apps"
		{
			"1284210"		"70000000000"
		}
	}
}"#;

    #[test]
    fn extracts_all_library_paths() {
        assert_eq!(
            library_folders(SAMPLE),
            vec![
                PathBuf::from(r"C:\Program Files (x86)\Steam"),
                PathBuf::from(r"D:\SteamLibrary"),
            ]
        );
    }

    #[test]
    fn empty_input_yields_no_paths() {
        assert!(library_folders("").is_empty());
    }
}
