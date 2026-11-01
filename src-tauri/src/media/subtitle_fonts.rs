//! Private subtitle fonts, embedded in every build (NSIS, deb and AppImage alike).
//!
//! A fresh directory per encode avoids changing the user's font settings or process-wide
//! environment. Nothing is downloaded at runtime and system fonts are not required.
use std::path::{Path, PathBuf};

pub struct SubtitleFonts {
    dir: PathBuf,
}

impl SubtitleFonts {
    pub fn prepare() -> std::io::Result<Self> {
        let dir =
            std::env::temp_dir().join(format!("vrcast-fonts-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir(&dir)?;
        let fonts = Self { dir };
        std::fs::write(
            fonts.dir.join("DejaVuSans.ttf"),
            include_bytes!("../../resources/fonts/DejaVuSans.ttf"),
        )?;
        std::fs::write(
            fonts.dir.join("LICENSE"),
            include_bytes!("../../resources/fonts/LICENSE"),
        )?;
        std::fs::create_dir(fonts.dir.join("cache"))?;
        let font_dir = xml_path(&fonts.dir);
        let cache_dir = xml_path(&fonts.dir.join("cache"));
        std::fs::write(
            fonts.config_path(),
            format!(
            "<?xml version=\"1.0\"?>\n<!DOCTYPE fontconfig SYSTEM \"urn:fontconfig:fonts.dtd\">\n\
             <fontconfig><dir>{font_dir}</dir><cachedir>{cache_dir}</cachedir></fontconfig>\n"
        ),
        )?;
        Ok(fonts)
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn config_path(&self) -> PathBuf {
        self.dir.join("fonts.conf")
    }
}

impl Drop for SubtitleFonts {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn xml_path(path: &Path) -> String {
    path.to_string_lossy()
        .replace('\\', "/")
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fonts_are_embedded_and_the_private_directory_is_removed() {
        let first = SubtitleFonts::prepare().unwrap();
        let second = SubtitleFonts::prepare().unwrap();
        assert_ne!(first.dir(), second.dir());
        let dir = first.dir().to_owned();
        assert_eq!(
            std::fs::read(dir.join("DejaVuSans.ttf")).unwrap(),
            include_bytes!("../../resources/fonts/DejaVuSans.ttf")
        );
        let conf = std::fs::read_to_string(first.config_path()).unwrap();
        assert!(conf.contains(&xml_path(&dir)));
        assert!(!conf.contains("/etc/fonts"));
        drop(first);
        assert!(!dir.exists());
        assert!(second.dir().exists());
    }

    #[test]
    fn configuration_paths_are_xml_not_markup() {
        assert_eq!(
            xml_path(Path::new("C:/films & clips/<fonts>")),
            "C:/films &amp; clips/&lt;fonts&gt;"
        );
    }
}
