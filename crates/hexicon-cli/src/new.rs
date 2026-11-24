//! `hexicon new`: a game to start from, in a new folder: a manifest
//! (`hexicon.toml`) and a first source (`start.hxn`) on Hexad, which
//! build and play as they are.

use std::path::Path;
use std::process::ExitCode;

/// The game's title, from its folder's name: `my-game` is "My Game".
fn title_of(name: &str) -> String {
    let words: Vec<String> = name
        .split(['-', '_', ' '])
        .filter(|word| !word.is_empty())
        .map(|word| {
            let mut chars = word.chars();
            chars.next().map_or_else(String::new, |first| {
                first.to_uppercase().chain(chars).collect()
            })
        })
        .collect();
    if words.is_empty() {
        "My Game".to_owned()
    } else {
        words.join(" ")
    }
}

/// A new IFID: a random UUID, as the IFID standard has them.
fn new_ifid() -> String {
    use std::hash::{BuildHasher, Hasher};
    let mut bytes = [0u8; 16];
    for chunk in bytes.chunks_mut(8) {
        let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
        hasher.write_u64(0);
        chunk.copy_from_slice(&hasher.finish().to_le_bytes());
    }
    // Version 4 (random), variant 1.
    bytes[6] = (bytes[6] & 0x0F) | 0x40;
    bytes[8] = (bytes[8] & 0x3F) | 0x80;
    let hex: String = bytes.iter().map(|b| format!("{b:02X}")).collect();
    format!(
        "{}-{}-{}-{}-{}",
        &hex[..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..]
    )
}

/// A string as TOML writes one, in double quotes.
fn toml_string(text: &str) -> String {
    format!("\"{}\"", text.replace('\\', "\\\\").replace('"', "\\\""))
}

/// A string as Hexicon writes one, in single quotes.
fn source_string(text: &str) -> String {
    format!("'{}'", text.replace('\\', "\\\\").replace('\'', "\\'"))
}

fn manifest(title: &str, image: &str) -> String {
    format!(
        "# {title}, a game for Hexicon.\n\
         \n\
         [game]\n\
         name = {}\n\
         libraries = [\"system\", \"hexad\"]\n\
         sources = [\"start\"]\n\
         image = {}\n\
         \n\
         [defines]\n\
         LANGUAGE = \"english\"\n",
        toml_string(title),
        toml_string(image)
    )
}

fn start(title: &str, ifid: &str) -> String {
    format!(
        "// Where {title} begins.\n\
         \n\
         import hexad;\n\
         \n\
         // What the game is called, and who wrote it.\n\
         versionInfo: GameID\n\
         \x20   IFID = '{ifid}'\n\
         \x20   name = {}\n\
         \x20   byline = 'by the author'\n\
         \x20   version = '1'\n\
         ;\n\
         \n\
         // How the game begins: who the player is.\n\
         gameMain: GameMainDef\n\
         \x20   initialPlayerChar = me\n\
         ;\n\
         \n\
         // The first room, and what is in it.\n\
         startRoom: Room 'Start Room'\n\
         \x20   \"This is where the game begins. \"\n\
         ;\n\
         \n\
         + note: Thing 'note'\n\
         \x20   \"It says: <q>Welcome to <<versionInfo.name>>.</q> \"\n\
         ;\n\
         \n\
         // The player, who starts in the room.\n\
         + me: Player 'you'\n\
         ;\n",
        source_string(title)
    )
}

/// Make the game in `folder`: what is written, or what is wrong.
pub(crate) fn create(folder: &Path) -> Result<(String, String), String> {
    let shown = folder.display();
    if folder.exists()
        && std::fs::read_dir(folder)
            .map(|mut entries| entries.next().is_some())
            .unwrap_or(true)
    {
        return Err(format!(
            "{shown}: already exists and is not empty; start a game in a new folder"
        ));
    }
    let name = folder
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let title = title_of(&name);
    let image = format!("{}.t3", if name.is_empty() { "game" } else { &name });
    std::fs::create_dir_all(folder).map_err(|e| format!("{shown}: {e}"))?;
    for (file, text) in [
        ("hexicon.toml", manifest(&title, &image)),
        ("start.hxn", start(&title, &new_ifid())),
    ] {
        let path = folder.join(file);
        std::fs::write(&path, text).map_err(|e| format!("{}: {e}", path.display()))?;
    }
    Ok((title, image))
}

pub(crate) fn run(args: &[&str], usage: &str) -> ExitCode {
    let [folder] = args else {
        eprint!("{usage}");
        return ExitCode::from(2);
    };
    match create(Path::new(folder)) {
        Ok((title, image)) => {
            println!(
                "Started {title} in {folder}: hexicon.toml and start.hxn.\n\
                 To build it and play it:\n\
                 \n\
                 \x20   cd {folder}\n\
                 \x20   hexicon build\n\
                 \x20   hexicon run {image}"
            );
            ExitCode::SUCCESS
        }
        Err(message) => {
            eprintln!("hexicon: {message}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_title_comes_from_the_folders_name() {
        assert_eq!(title_of("my-game"), "My Game");
        assert_eq!(title_of("the_lost_key"), "The Lost Key");
        assert_eq!(title_of("Cave"), "Cave");
        assert_eq!(title_of(""), "My Game");
    }

    #[test]
    fn an_ifid_is_a_random_uuid() {
        let ifid = new_ifid();
        assert_eq!(ifid.len(), 36);
        assert_eq!(&ifid[14..15], "4");
        assert!(matches!(&ifid[19..20], "8" | "9" | "A" | "B"));
        assert_ne!(ifid, new_ifid());
    }

    #[test]
    fn a_new_game_builds_as_it_is() {
        let folder = std::env::temp_dir().join(format!("hexicon-new-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        let game = folder.join("my-game");
        let (title, image) = create(&game).unwrap();
        assert_eq!((title.as_str(), image.as_str()), ("My Game", "my-game.t3"));
        // Not over a folder with something in it.
        assert!(create(&game).is_err());

        let game_dir = game.to_string_lossy().into_owned();
        let out = game.join(&image).to_string_lossy().into_owned();
        let _ = crate::build::run(&["-f", &game_dir, "-o", &out], "");
        let built = std::fs::metadata(&out).map(|meta| meta.len()).unwrap_or(0);
        let _ = std::fs::remove_dir_all(&folder);
        assert!(built > 0, "no image was built");
    }
}
