//! Our representation of all the supported compression formats.

use std::{ffi::OsStr, fmt, path::Path};

use bstr::ByteSlice;
use CompressionFormat::*;

use crate::{
    error::{Error, FinalError, Result},
    utils::logger::warning,
};

pub const SUPPORTED_EXTENSIONS: &[&str] = &[
    "tar",
    "zip",
    "bz",
    "bz2",
    "gz",
    "lz4",
    "xz",
    "lzma",
    "lz",
    "sz",
    "zst",
    #[cfg(feature = "unrar")]
    "rar",
    "7z",
    "br",
];

pub const SUPPORTED_ALIASES: &[&str] = &["tgz", "tbz", "tlz4", "txz", "tlzma", "tsz", "tzst", "tlz"];

#[cfg(not(feature = "unrar"))]
pub const PRETTY_SUPPORTED_EXTENSIONS: &str = "tar, zip, bz, bz2, bz3, gz, lz4, xz, lzma, lz, sz, zst, 7z";
#[cfg(feature = "unrar")]
pub const PRETTY_SUPPORTED_EXTENSIONS: &str = "tar, zip, bz, bz2, bz3, gz, lz4, xz, lzma, lz, sz, zst, rar, 7z";

pub const PRETTY_SUPPORTED_ALIASES: &str = "tgz, tbz, tlz4, txz, tlzma, tsz, tzst, tlz";

/// A wrapper around `CompressionFormat` that allows combinations like `tgz`
#[derive(Debug, Clone)]
// Keep `PartialEq` only for testing because two formats are the same even if
// their `display_text` does not match (beware of aliases)
#[cfg_attr(test, derive(PartialEq))]
// Should only be built with constructors
#[non_exhaustive]
pub struct Extension {
    /// One extension like "tgz" can be made of multiple CompressionFormats ([Tar, Gz])
    pub compression_formats: &'static [CompressionFormat],
    /// The input text for this extension, like "tgz", "tar" or "xz"
    display_text: String,
}

impl Extension {
    /// # Panics:
    ///   Will panic if `formats` is empty
    pub fn new(formats: &'static [CompressionFormat], text: impl ToString) -> Self {
        assert!(!formats.is_empty());
        Self {
            compression_formats: formats,
            display_text: text.to_string(),
        }
    }

    /// Checks if the first format in `compression_formats` is an archive
    pub fn is_archive(&self) -> bool {
        // Index Safety: we check that `compression_formats` is not empty in `Self::new`
        self.compression_formats[0].archive_format()
    }
}

impl fmt::Display for Extension {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        self.display_text.fmt(f)
    }
}

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
/// Accepted extensions for input and output
pub enum CompressionFormat {
    /// .gz
    Gzip,
    /// .bz .bz2
    Bzip,
    /// .bz3
    Bzip3,
    /// .lz4
    Lz4,
    /// .xz
    Xz,
    /// .lzma
    Lzma,
    /// .lzip
    Lzip,
    /// .sz
    Snappy,
    /// tar, tgz, tbz, tbz2, tbz3, txz, tlz4, tlzma, tsz, tzst
    Tar,
    /// .zst
    Zstd,
    /// .zip
    Zip,
    /// .rar
    Rar,
    /// .7z
    SevenZip,
    /// .br
    Brotli,
}

impl CompressionFormat {
    pub fn archive_format(&self) -> bool {
        // Keep this match without a wildcard `_` so we never forget to update it
        match self {
            Tar | Zip | Rar | SevenZip => true,
            Bzip | Bzip3 | Lz4 | Lzma | Xz | Lzip | Snappy | Zstd | Brotli | Gzip => false,
        }
    }
}

fn to_extension(ext: &[u8]) -> Option<Extension> {
    Some(Extension::new(
        match ext {
            b"tar" => &[Tar],
            b"tgz" => &[Tar, Gzip],
            b"tbz" | b"tbz2" => &[Tar, Bzip],
            b"tbz3" => &[Tar, Bzip3],
            b"tlz4" => &[Tar, Lz4],
            b"txz" => &[Tar, Xz],
            b"tlzma" => &[Tar, Lzma],
            b"tlz" => &[Tar, Lzip],
            b"tsz" => &[Tar, Snappy],
            b"tzst" => &[Tar, Zstd],
            b"zip" => &[Zip],
            b"bz" | b"bz2" => &[Bzip],
            b"bz3" => &[Bzip3],
            b"gz" => &[Gzip],
            b"lz4" => &[Lz4],
            b"xz" => &[Xz],
            b"lzma" => &[Lzma],
            b"lz" => &[Lzip],
            b"sz" => &[Snappy],
            b"zst" => &[Zstd],
            b"rar" => &[Rar],
            b"7z" => &[SevenZip],
            b"br" => &[Brotli],
            _ => return None,
        },
        ext.to_str_lossy(),
    ))
}

fn split_extension_at_end(name: &[u8]) -> Option<(&[u8], Extension)> {
    let (new_name, ext) = name.rsplit_once_str(b".")?;
    if matches!(new_name, b"" | b"." | b"..") {
        return None;
    }
    let ext = to_extension(ext)?;
    Some((new_name, ext))
}

pub fn parse_format_flag(input: &OsStr) -> crate::Result<Vec<Extension>> {
    let format = input.as_encoded_bytes();

    let format = std::str::from_utf8(format).map_err(|_| Error::InvalidFormatFlag {
        text: input.to_owned(),
        reason: "Invalid UTF-8.".to_string(),
    })?;

    let extensions: Vec<Extension> = format
        .split('.')
        .filter(|extension| !extension.is_empty())
        .map(|extension| {
            to_extension(extension.as_bytes()).ok_or_else(|| Error::InvalidFormatFlag {
                text: input.to_owned(),
                reason: format!("Unsupported extension '{extension}'"),
            })
        })
        .collect::<crate::Result<_>>()?;

    if extensions.is_empty() {
        return Err(Error::InvalidFormatFlag {
            text: input.to_owned(),
            reason: "Parsing got an empty list of extensions.".to_string(),
        });
    }

    Ok(extensions)
}

/// Extracts extensions from a path.
///
/// Returns both the remaining path and the list of extension objects.
pub fn separate_known_extensions_from_name(path: &Path) -> Result<(&Path, Vec<Extension>)> {
    let mut extensions = vec![];

    let Some(mut name) = path.file_name().and_then(<[u8] as ByteSlice>::from_os_str) else {
        return Ok((path, extensions));
    };

    while let Some((new_name, extension)) = split_extension_at_end(name) {
        name = new_name;
        extensions.insert(0, extension);
        if extensions[0].is_archive() {
            if let Some((_, misplaced_extension)) = split_extension_at_end(name) {
                let mut error = FinalError::with_title("File extensions are invalid for operation").detail(format!(
                    "The archive extension '.{}' can only be placed at the start of the extension list",
                    extensions[0].display_text,
                ));

                if misplaced_extension.compression_formats == extensions[0].compression_formats {
                    error = error.detail(format!(
                        "File: '{path:?}' contains '.{}' and '.{}'",
                        misplaced_extension.display_text, extensions[0].display_text,
                    ));
                }

                return Err(error
                    .hint("You can use `--format` to specify what format to use, examples:")
                    .hint("  ouch compress file.zip.zip file --format zip")
                    .hint("  ouch decompress file --format zst")
                    .hint("  ouch list archive --format tar.gz")
                    .into());
            }
            break;
        }
    }

    if let Ok(name) = name.to_str() {
        let file_stem = name.trim_matches('.');
        if SUPPORTED_EXTENSIONS.contains(&file_stem) || SUPPORTED_ALIASES.contains(&file_stem) {
            warning(format!(
                "Received a file with name '{file_stem}', but {file_stem} was expected as the extension"
            ));
        }
    }

    Ok((name.to_path().unwrap(), extensions))
}

/// Extracts extensions from a path, return only the list of extension objects
pub fn extensions_from_path(path: &Path) -> Result<Vec<Extension>> {
    separate_known_extensions_from_name(path).map(|(_, extensions)| extensions)
}

/// Panics if formats has an empty list of compression formats
pub fn split_first_compression_format(formats: &[Extension]) -> (CompressionFormat, Vec<CompressionFormat>) {
    let mut extensions: Vec<CompressionFormat> = flatten_compression_formats(formats);
    let first_extension = extensions.remove(0);
    (first_extension, extensions)
}

pub fn flatten_compression_formats(extensions: &[Extension]) -> Vec<CompressionFormat> {
    extensions
        .iter()
        .flat_map(|extension| extension.compression_formats.iter())
        .copied()
        .collect()
}

/// Builds a suggested output file in scenarios where the user tried to compress
/// a folder into a non-archive compression format, for error message purposes
///
/// E.g.: `build_suggestion("file.bz.xz", ".tar")` results in `Some("file.tar.bz.xz")`
pub fn build_archive_file_suggestion(path: &Path, suggested_extension: &str) -> Option<String> {
    let path = path.to_string_lossy();
    let mut rest = &*path;
    let mut position_to_insert = 0;

    // Walk through the path to find the first supported compression extension
    while let Some(pos) = rest.find('.') {
        // Use just the text located after the dot we found
        rest = &rest[pos + 1..];
        position_to_insert += pos + 1;

        // If the string contains more chained extensions, clip to the immediate one
        let maybe_extension = {
            let idx = rest.find('.').unwrap_or(rest.len());
            &rest[..idx]
        };

        // If the extension we got is a supported extension, generate the suggestion
        // at the position we found
        if SUPPORTED_EXTENSIONS.contains(&maybe_extension) || SUPPORTED_ALIASES.contains(&maybe_extension) {
            let mut path = path.to_string();
            path.insert_str(position_to_insert - 1, suggested_extension);

            return Some(path);
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extensions_from_path() {
        let path = Path::new("bolovo.tar.gz");

        let extensions = extensions_from_path(path).unwrap();
        let formats = flatten_compression_formats(&extensions);

        assert_eq!(formats, vec![Tar, Gzip]);
    }

    #[test]
    /// Test extension parsing for input/output files
    fn test_separate_known_extensions_from_name() {
        assert_eq!(
            separate_known_extensions_from_name("file".as_ref()).unwrap(),
            ("file".as_ref(), vec![])
        );
        assert_eq!(
            separate_known_extensions_from_name("tar".as_ref()).unwrap(),
            ("tar".as_ref(), vec![])
        );
        assert_eq!(
            separate_known_extensions_from_name(".tar".as_ref()).unwrap(),
            (".tar".as_ref(), vec![])
        );
        assert_eq!(
            separate_known_extensions_from_name("file.tar".as_ref()).unwrap(),
            ("file".as_ref(), vec![Extension::new(&[Tar], "tar")])
        );
        assert_eq!(
            separate_known_extensions_from_name("file.tar.gz".as_ref()).unwrap(),
            (
                "file".as_ref(),
                vec![Extension::new(&[Tar], "tar"), Extension::new(&[Gzip], "gz")]
            )
        );
        assert_eq!(
            separate_known_extensions_from_name(".tar.gz".as_ref()).unwrap(),
            (".tar".as_ref(), vec![Extension::new(&[Gzip], "gz")])
        );
    }

    #[test]
    /// Test extension parsing of `--format FORMAT`
    fn test_parse_of_format_flag() {
        assert_eq!(
            parse_format_flag(OsStr::new("tar")).unwrap(),
            vec![Extension::new(&[Tar], "tar")]
        );
        assert_eq!(
            parse_format_flag(OsStr::new(".tar")).unwrap(),
            vec![Extension::new(&[Tar], "tar")]
        );
        assert_eq!(
            parse_format_flag(OsStr::new("tar.gz")).unwrap(),
            vec![Extension::new(&[Tar], "tar"), Extension::new(&[Gzip], "gz")]
        );
        assert_eq!(
            parse_format_flag(OsStr::new(".tar.gz")).unwrap(),
            vec![Extension::new(&[Tar], "tar"), Extension::new(&[Gzip], "gz")]
        );
        assert_eq!(
            parse_format_flag(OsStr::new("..tar..gz.....")).unwrap(),
            vec![Extension::new(&[Tar], "tar"), Extension::new(&[Gzip], "gz")]
        );

        assert!(parse_format_flag(OsStr::new("../tar.gz")).is_err());
        assert!(parse_format_flag(OsStr::new("targz")).is_err());
        assert!(parse_format_flag(OsStr::new("tar.gz.unknown")).is_err());
        assert!(parse_format_flag(OsStr::new(".tar.gz.unknown")).is_err());
        assert!(parse_format_flag(OsStr::new(".tar.!@#.gz")).is_err());
    }

    #[test]
    fn builds_suggestion_correctly() {
        assert_eq!(build_archive_file_suggestion(Path::new("linux.png"), ".tar"), None);
        assert_eq!(
            build_archive_file_suggestion(Path::new("linux.xz.gz.zst"), ".tar").unwrap(),
            "linux.tar.xz.gz.zst"
        );
        assert_eq!(
            build_archive_file_suggestion(Path::new("linux.pkg.xz.gz.zst"), ".tar").unwrap(),
            "linux.pkg.tar.xz.gz.zst"
        );
        assert_eq!(
            build_archive_file_suggestion(Path::new("linux.pkg.zst"), ".tar").unwrap(),
            "linux.pkg.tar.zst"
        );
        assert_eq!(
            build_archive_file_suggestion(Path::new("linux.pkg.info.zst"), ".tar").unwrap(),
            "linux.pkg.info.tar.zst"
        );
    }

    #[test]
    fn test_extension_parsing_with_multiple_archive_formats() {
        assert!(separate_known_extensions_from_name("file.tar.zip".as_ref()).is_err());
        assert!(separate_known_extensions_from_name("file.7z.zst.zip.lz4".as_ref()).is_err());
    }
}
