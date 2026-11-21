//! Bundling resources into an image, as the reference's `t3make` does
//! after it has written the image: pictures, sounds, and other files the
//! program asks for by name.

use hexicon_compiler::{DirEntry, Resource, ResourceSpec, expand_resources};

/// The files the resources name, with what is in each folder.
pub fn list(specs: &[ResourceSpec]) -> Vec<Resource> {
    expand_resources(specs, &mut list_dir)
}

/// What is in a folder, in the order the system lists it, or nothing if
/// the path is not a folder.
fn list_dir(path: &str) -> Option<Vec<DirEntry>> {
    if !std::fs::metadata(path).ok()?.is_dir() {
        return None;
    }
    let mut entries = Vec::new();
    for entry in std::fs::read_dir(path).ok()?.flatten() {
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        let name = entry.file_name().to_string_lossy().into_owned();
        entries.push(DirEntry {
            hidden: is_hidden(&name, &meta),
            name,
            is_dir: meta.is_dir(),
        });
    }
    Some(entries)
}

/// Whether a file is hidden or the system's, which the reference leaves
/// out of a folder's resources.
#[cfg(windows)]
fn is_hidden(_name: &str, meta: &std::fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    const HIDDEN: u32 = 0x2;
    const SYSTEM: u32 = 0x4;
    meta.file_attributes() & (HIDDEN | SYSTEM) != 0
}

#[cfg(not(windows))]
fn is_hidden(name: &str, _meta: &std::fs::Metadata) -> bool {
    name.starts_with('.')
}

/// The block that ends an image.
const EOF_BLOCK: [u8; 10] = *b"EOF \0\0\0\0\x01\0";

/// Add the resources to an image, in place of its end, with a new end
/// after them. In a build for debugging, the image gets only links to
/// the files (`MREL`), and otherwise the files themselves (`MRES`), each
/// named with its bytes inverted. `image_name` is for the messages, and
/// `step` is told of each file as it is copied.
pub fn bundle(
    image: &mut Vec<u8>,
    image_name: &str,
    resources: &[Resource],
    link: bool,
    step: &mut dyn FnMut(&str),
) -> Result<(), String> {
    if image.len() < 10 || &image[image.len() - 10..image.len() - 6] != b"EOF " {
        return Err(format!("{image_name}: unexpected end of file"));
    }
    let mut block = Vec::new();
    block.extend_from_slice(&(resources.len() as u16).to_le_bytes());
    let too_long = |resource: &Resource| {
        format!(
            "{image_name}: resource name \"{}\" for file \"{}\" is too long",
            resource.name, resource.file
        )
    };
    if link {
        for resource in resources {
            if resource.name.len() > 255 {
                return Err(too_long(resource));
            }
            if resource.file.len() > 255 {
                return Err(format!(
                    "{image_name}: filename \"{}\" for resource link \"{}\" is too long",
                    resource.file, resource.name
                ));
            }
            block.push(resource.name.len() as u8);
            block.extend_from_slice(resource.name.as_bytes());
            block.push(resource.file.len() as u8);
            block.extend_from_slice(resource.file.as_bytes());
        }
    } else {
        // The table of contents: where each file's bytes start in the
        // block, how many there are, and the name.
        let mut contents = Vec::with_capacity(resources.len());
        let mut offset = 2 + resources
            .iter()
            .map(|resource| 9 + resource.name.len())
            .sum::<usize>();
        for resource in resources {
            if resource.name.len() > 255 {
                return Err(too_long(resource));
            }
            let Ok(data) = std::fs::read(&resource.file) else {
                return Err(format!(
                    "{image_name}: cannot open resource file \"{}\"",
                    resource.file
                ));
            };
            block.extend_from_slice(&(offset as u32).to_le_bytes());
            block.extend_from_slice(&(data.len() as u32).to_le_bytes());
            block.push(resource.name.len() as u8);
            block.extend(resource.name.bytes().map(|byte| byte ^ 0xff));
            offset += data.len();
            contents.push(data);
        }
        for (resource, data) in resources.iter().zip(contents) {
            step(&format!("+ {} ({})", resource.file, resource.name));
            block.extend_from_slice(&data);
        }
    }
    // The block takes the place of the end, which follows it.
    image.truncate(image.len() - 10);
    image.extend_from_slice(if link { b"MREL" } else { b"MRES" });
    image.extend_from_slice(&(block.len() as u32).to_le_bytes());
    image.extend_from_slice(&0u16.to_le_bytes());
    image.extend_from_slice(&block);
    image.extend_from_slice(&EOF_BLOCK);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An image with nothing in it but its header and its end.
    fn empty_image() -> Vec<u8> {
        let mut image = b"T3-image\r\n\x1a".to_vec();
        image.extend_from_slice(&1u16.to_le_bytes());
        image.extend_from_slice(&[0; 32]);
        image.extend_from_slice(b"Sat Nov 21 12:00:00 2026");
        image.extend_from_slice(&EOF_BLOCK);
        image
    }

    #[test]
    fn resources_go_before_the_end_of_the_image() {
        let dir = std::env::temp_dir().join(format!("hexicon-res-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("a.txt");
        std::fs::write(&file, b"hello").unwrap();
        let file = file.to_string_lossy().into_owned();
        let resources = [Resource {
            file: file.clone(),
            name: "texts/a.txt".to_owned(),
        }];

        // The files themselves, with a table of where each is.
        let mut image = empty_image();
        let mut steps = Vec::new();
        bundle(&mut image, "game.t3", &resources, false, &mut |s| {
            steps.push(s.to_owned());
        })
        .unwrap();
        assert_eq!(steps, [format!("+ {file} (texts/a.txt)")]);
        let header = empty_image().len() - 10;
        assert_eq!(&image[header..header + 10], b"MRES\x1b\0\0\0\0\0");
        let block = &image[header + 10..image.len() - 10];
        assert_eq!(&block[..2], &1u16.to_le_bytes());
        // The data starts after the count and the one entry.
        assert_eq!(&block[2..10], &[22, 0, 0, 0, 5, 0, 0, 0]);
        assert_eq!(block[10], 11);
        let name: Vec<u8> = block[11..22].iter().map(|b| b ^ 0xff).collect();
        assert_eq!(name, b"texts/a.txt");
        assert_eq!(&block[22..], b"hello");
        assert!(image.ends_with(&EOF_BLOCK));

        // In a build for debugging, links to them.
        let mut image = empty_image();
        bundle(&mut image, "game.t3", &resources, true, &mut |_| {}).unwrap();
        let block = &image[header + 10..image.len() - 10];
        assert_eq!(&image[header..header + 4], b"MREL");
        let mut expected = vec![1, 0, 11];
        expected.extend_from_slice(b"texts/a.txt");
        expected.push(file.len() as u8);
        expected.extend_from_slice(file.as_bytes());
        assert_eq!(block, expected);

        // A file that is not there stops the build.
        let missing = [Resource {
            file: "no-such-file.png".to_owned(),
            name: "x.png".to_owned(),
        }];
        let mut image = empty_image();
        assert_eq!(
            bundle(&mut image, "game.t3", &missing, false, &mut |_| {}),
            Err("game.t3: cannot open resource file \"no-such-file.png\"".to_owned())
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
