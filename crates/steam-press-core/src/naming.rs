//! Portable, SuperSteamPacker-compatible output naming.

use crate::platform::Platform;

const WINDOWS_ILLEGAL: &[char] = &['<', '>', ':', '"', '/', '\\', '|', '?', '*'];

fn sanitize(name: &str, spaces_to_dots: bool) -> String {
    let mut out = String::with_capacity(name.len());
    for character in name.chars() {
        if character == ' ' {
            if spaces_to_dots {
                out.push('.');
            } else {
                out.push(' ');
            }
        } else if character == '\''
            || !character.is_ascii()
            || character.is_control()
            || WINDOWS_ILLEGAL.contains(&character)
        {
            continue;
        } else {
            out.push(character);
        }
    }
    out.trim_end_matches(['.', ' ']).to_string()
}

/// Name used for the archive stem: spaces become dots.
pub fn sanitize_game_name(name: &str) -> String {
    sanitize(name, true)
}

/// Name used for `steamapps/common/<installdir>`: spaces are kept.
pub fn sanitize_install_dir(name: &str) -> String {
    sanitize(name, false)
}

/// `<Game>.Build.<build>.<OS>.<branch>`, exactly like SuperSteamPacker.
pub fn output_stem(game_name: &str, build_id: &str, platform: Platform, branch: &str) -> String {
    let build = if build_id.trim().is_empty() {
        "Unknown"
    } else {
        build_id.trim()
    };
    let branch = if branch.trim().is_empty() {
        "Public"
    } else {
        branch.trim()
    };
    format!(
        "{}.Build.{}.{}.{}",
        sanitize_game_name(game_name),
        build,
        platform.os_tag(),
        branch
    )
}

pub fn archive_stem(game_name: &str, build_id: &str, platform: Platform, branch: &str) -> String {
    output_stem(game_name, build_id, platform, branch)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitizes_like_supersteampacker() {
        assert_eq!(sanitize_game_name("Half-Life 2"), "Half-Life.2");
        assert_eq!(sanitize_game_name("Assassin's Creed"), "Assassins.Creed");
        assert_eq!(sanitize_game_name("Who's Your Daddy?!"), "Whos.Your.Daddy!");
        assert_eq!(sanitize_game_name("S.T.A.L.K.E.R."), "S.T.A.L.K.E.R");
        assert_eq!(sanitize_game_name("Café Game™"), "Caf.Game");
    }

    #[test]
    fn install_dir_keeps_spaces() {
        assert_eq!(sanitize_install_dir("The Crust"), "The Crust");
        assert_eq!(sanitize_install_dir("Assassin's Creed"), "Assassins Creed");
        assert_eq!(sanitize_install_dir("Game/Name\\Test"), "GameNameTest");
    }

    #[test]
    fn builds_output_stem() {
        assert_eq!(
            output_stem("Portal 2", "5781", Platform::Linux64, "public"),
            "Portal.2.Build.5781.Linux64.public"
        );
        assert_eq!(
            output_stem("Game", "", Platform::Win64, "beta"),
            "Game.Build.Unknown.Win64.beta"
        );
    }
}
