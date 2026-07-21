use std::{
    fs::{self, File, Metadata},
    io::{Read, Take},
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};
use ttf_parser::{Face, Permissions, name_id};

use crate::FontSlant;

pub(crate) const MAX_EMBEDDED_FONT_BYTES: usize = 32 * 1024 * 1024;

/// One bounded, authorized, immutable read of a source OpenType font.
///
/// The bytes are private so callers cannot change the identity that was
/// validated at construction time. Durable Prism projects store these exact
/// bytes through the existing content-addressed asset transaction.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FontSourceSnapshot {
    canonical_path: PathBuf,
    content_hash: String,
    bytes: Vec<u8>,
    pub(crate) family: String,
    pub(crate) style: String,
    pub(crate) weight: u16,
    pub(crate) slant: FontSlant,
    pub(crate) subset_allowed: bool,
}

/// Verified full-font bytes loaded without requiring a materialized asset path.
///
/// Durable read-only inspection uses this representation directly from the
/// immutable SQLite asset blob so it never publishes a cache file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VerifiedFontSource {
    content_hash: String,
    bytes: Vec<u8>,
    pub(crate) family: String,
    pub(crate) style: String,
    pub(crate) weight: u16,
    pub(crate) slant: FontSlant,
    pub(crate) subset_allowed: bool,
}

impl FontSourceSnapshot {
    pub fn read(path: &Path) -> Result<Self> {
        Self::read_inner(path, None)
    }

    fn read_inner(path: &Path, expected_hash: Option<&str>) -> Result<Self> {
        let file = open_no_follow(path)?;
        let opened_metadata = file.metadata()?;
        require_regular_metadata(&opened_metadata, path)?;
        let bytes = read_bounded(file, &opened_metadata, path)?;
        let canonical_path = verify_unchanged_path(path, &opened_metadata)?;
        let verified = VerifiedFontSource::from_bytes(bytes, expected_hash, path)?;
        Ok(Self {
            canonical_path,
            content_hash: verified.content_hash,
            bytes: verified.bytes,
            family: verified.family,
            style: verified.style,
            weight: verified.weight,
            slant: verified.slant,
            subset_allowed: verified.subset_allowed,
        })
    }

    pub(crate) fn read_verified(path: &Path, expected_hash: &str) -> Result<Self> {
        if !path.is_absolute() {
            bail!("embedded font source path is not materialized safely");
        }
        validate_content_hash(expected_hash)?;
        Self::read_inner(path, Some(expected_hash))
    }

    pub fn canonical_path(&self) -> &Path {
        &self.canonical_path
    }

    pub fn content_hash(&self) -> &str {
        &self.content_hash
    }

    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub fn len(&self) -> usize {
        self.bytes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    pub fn subset_allowed(&self) -> bool {
        self.subset_allowed
    }
}

impl VerifiedFontSource {
    pub(crate) fn from_embedded_bytes(bytes: Vec<u8>, expected_hash: &str) -> Result<Self> {
        validate_content_hash(expected_hash)?;
        Self::from_bytes(bytes, Some(expected_hash), Path::new("embedded font asset"))
    }

    fn from_bytes(bytes: Vec<u8>, expected_hash: Option<&str>, label: &Path) -> Result<Self> {
        if bytes.is_empty() {
            bail!("font data cannot be empty");
        }
        if bytes.len() > MAX_EMBEDDED_FONT_BYTES {
            bail!("font exceeds Prism's 32 MiB embedded-font limit");
        }
        let content_hash = sha256_hex(&bytes);
        if expected_hash.is_some_and(|expected| expected != content_hash) {
            bail!("embedded font source bytes do not match their content identity");
        }
        let face = Face::parse(&bytes, 0)
            .with_context(|| format!("{} is not a supported OpenType font", label.display()))?;
        require_editable_embedding(&face)?;
        let family = font_name(&face, &[name_id::TYPOGRAPHIC_FAMILY, name_id::FAMILY])
            .unwrap_or_else(|| "Imported Font".into());
        let style = font_name(&face, &[name_id::TYPOGRAPHIC_SUBFAMILY, name_id::SUBFAMILY])
            .unwrap_or_else(|| "Regular".into());
        let slant = if face.is_italic() {
            FontSlant::Italic
        } else if face.is_oblique() {
            FontSlant::Oblique
        } else {
            FontSlant::Normal
        };
        let weight = face.weight().to_number();
        let subset_allowed = face.is_subsetting_allowed();
        Ok(Self {
            content_hash,
            bytes,
            family,
            style,
            weight,
            slant,
            subset_allowed,
        })
    }

    pub fn content_hash(&self) -> &str {
        &self.content_hash
    }

    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub fn len(&self) -> usize {
        self.bytes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    pub fn subset_allowed(&self) -> bool {
        self.subset_allowed
    }
}

fn validate_content_hash(expected_hash: &str) -> Result<()> {
    if expected_hash.len() != 64
        || !expected_hash
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        bail!("embedded font source has an invalid content identity");
    }
    Ok(())
}

fn read_bounded(file: File, before: &Metadata, path: &Path) -> Result<Vec<u8>> {
    if before.len() == 0 {
        bail!("font data cannot be empty");
    }
    if before.len() > MAX_EMBEDDED_FONT_BYTES as u64 {
        bail!("font exceeds Prism's 32 MiB embedded-font limit");
    }
    let expected_len = usize::try_from(before.len()).context("font length does not fit memory")?;
    let mut bytes = Vec::with_capacity(expected_len);
    let mut reader: Take<File> = file.take(MAX_EMBEDDED_FONT_BYTES as u64 + 1);
    reader.read_to_end(&mut bytes)?;
    let file = reader.into_inner();
    let after = file.metadata()?;
    if !stable_file(before, &after) || bytes.len() != expected_len {
        bail!("font source changed while reading {}", path.display());
    }
    if bytes.len() > MAX_EMBEDDED_FONT_BYTES {
        bail!("font exceeds Prism's 32 MiB embedded-font limit");
    }
    Ok(bytes)
}

fn verify_unchanged_path(path: &Path, opened: &Metadata) -> Result<PathBuf> {
    let current_file = open_no_follow(path)?;
    let current = current_file.metadata()?;
    require_regular_metadata(&current, path)?;
    if !stable_file(opened, &current) {
        bail!("font source changed while reading {}", path.display());
    }
    let canonical = fs::canonicalize(path)
        .with_context(|| format!("could not resolve font {}", path.display()))?;
    let canonical_metadata = fs::symlink_metadata(&canonical)?;
    if canonical_metadata.file_type().is_symlink() || !same_file(opened, &canonical_metadata) {
        bail!("font source path changed while reading {}", path.display());
    }
    Ok(canonical)
}

fn require_regular_metadata(metadata: &Metadata, path: &Path) -> Result<()> {
    if !metadata.is_file() {
        bail!("font source {} is not a regular file", path.display());
    }
    Ok(())
}

#[cfg(any(unix, windows))]
fn absolute_lexical_path(path: &Path) -> Result<PathBuf> {
    let absolute = if path.is_absolute() {
        path.to_owned()
    } else {
        std::env::current_dir()?.join(path)
    };
    let mut normalized = PathBuf::new();
    for component in absolute.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                bail!("font source path cannot contain parent-directory traversal");
            }
            _ => normalized.push(component.as_os_str()),
        }
    }
    Ok(normalized)
}

#[cfg(unix)]
fn open_no_follow(path: &Path) -> Result<File> {
    use std::{
        ffi::CString,
        os::{
            fd::{AsRawFd, FromRawFd},
            unix::ffi::OsStrExt,
        },
    };

    let absolute = absolute_lexical_path(path)?;
    let mut directory = File::open("/")?;
    let mut components = absolute
        .components()
        .filter_map(|component| match component {
            std::path::Component::Normal(name) => Some(name),
            _ => None,
        });
    let Some(mut name) = components.next() else {
        bail!("font source path does not name a file");
    };
    loop {
        let next = components.next();
        let component = CString::new(name.as_bytes()).context("font path contains a null byte")?;
        let flags = libc::O_RDONLY
            | libc::O_CLOEXEC
            | libc::O_NOFOLLOW
            | libc::O_NONBLOCK
            | if next.is_some() { libc::O_DIRECTORY } else { 0 };
        // SAFETY: `directory` owns a live directory descriptor, `component` is a
        // null-terminated component, and a successful descriptor is immediately
        // transferred into `File`. O_NOFOLLOW binds every traversed component.
        let descriptor = unsafe { libc::openat(directory.as_raw_fd(), component.as_ptr(), flags) };
        if descriptor < 0 {
            return Err(std::io::Error::last_os_error())
                .with_context(|| format!("could not securely open font {}", path.display()));
        }
        // SAFETY: `openat` returned a new owned descriptor above.
        let opened = unsafe { File::from_raw_fd(descriptor) };
        let Some(next_name) = next else {
            return Ok(opened);
        };
        directory = opened;
        name = next_name;
    }
}

#[cfg(windows)]
const WINDOWS_NO_FOLLOW_FLAGS: u32 =
    windows_sys::Win32::Storage::FileSystem::FILE_FLAG_OPEN_REPARSE_POINT;

#[cfg(windows)]
const _: () = assert!(
    WINDOWS_NO_FOLLOW_FLAGS & windows_sys::Win32::Storage::FileSystem::FILE_FLAG_OPEN_REPARSE_POINT
        != 0
);

#[cfg(all(test, windows))]
std::thread_local! {
    static WINDOWS_AFTER_SCAN_HOOK: std::cell::RefCell<Option<Box<dyn FnOnce()>>> =
        std::cell::RefCell::new(None);
}

#[cfg(windows)]
fn open_no_follow(path: &Path) -> Result<File> {
    use std::os::windows::fs::{MetadataExt, OpenOptionsExt};

    let absolute = absolute_lexical_path(path)?;
    reject_windows_reparse_components(&absolute)?;
    #[cfg(test)]
    WINDOWS_AFTER_SCAN_HOOK.with(|hook| {
        if let Some(hook) = hook.borrow_mut().take() {
            hook();
        }
    });
    let file = fs::OpenOptions::new()
        .read(true)
        .custom_flags(WINDOWS_NO_FOLLOW_FLAGS)
        .open(&absolute)
        .with_context(|| format!("could not securely open font {}", path.display()))?;
    if file.metadata()?.file_attributes()
        & windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT
        != 0
    {
        bail!("refusing reparse-point font source {}", path.display());
    }
    require_windows_final_path_matches(&absolute, &file)?;
    reject_windows_reparse_components(&absolute)?;
    Ok(file)
}

#[cfg(windows)]
fn require_windows_final_path_matches(intended: &Path, file: &File) -> Result<()> {
    use std::os::windows::io::AsRawHandle;

    let mut buffer = vec![0_u16; 512];
    let final_path = loop {
        let length = unsafe {
            windows_sys::Win32::Storage::FileSystem::GetFinalPathNameByHandleW(
                file.as_raw_handle(),
                buffer.as_mut_ptr(),
                buffer.len().try_into().unwrap_or(u32::MAX),
                0,
            )
        };
        if length == 0 {
            return Err(std::io::Error::last_os_error())
                .context("could not prove the final font source path");
        }
        let required = usize::try_from(length).context("final font path length overflowed")?;
        if required < buffer.len() {
            buffer.truncate(required);
            break String::from_utf16(&buffer).context("final font path is not valid UTF-16")?;
        }
        if required > 32_767 {
            bail!("final font handle path exceeds the Windows path limit");
        }
        buffer.resize(required.saturating_add(1), 0);
    };
    let intended = intended
        .to_str()
        .context("intended font path is not valid Unicode")?;
    let final_path = normalize_windows_proof_path(&final_path)?;
    let intended_path = normalize_windows_proof_path(intended)?;
    if !windows_proof_paths_match_normalized(&intended_path, &final_path) {
        bail!(
            "final font handle path {} does not match intended path {}",
            final_path,
            intended_path
        );
    }
    Ok(())
}

#[cfg(windows)]
fn windows_proof_paths_match(intended: &str, final_path: &str) -> Result<bool> {
    // Do not expand aliases on the intended side. GetFinalPathNameByHandleW's
    // normalized long path must prove the same spelling after prefix/separator
    // normalization. An 8.3 input therefore fails unless Windows itself reports
    // that exact alias as the final normalized handle path.
    let intended = normalize_windows_proof_path(intended)?;
    let final_path = normalize_windows_proof_path(final_path)?;
    Ok(windows_proof_paths_match_normalized(&intended, &final_path))
}

#[cfg(windows)]
fn windows_proof_paths_match_normalized(intended: &str, final_path: &str) -> bool {
    let final_wide: Vec<u16> = final_path.encode_utf16().collect();
    let intended_wide: Vec<u16> = intended.encode_utf16().collect();
    let comparison = unsafe {
        windows_sys::Win32::Globalization::CompareStringOrdinal(
            final_wide.as_ptr(),
            final_wide.len().try_into().unwrap_or(i32::MAX),
            intended_wide.as_ptr(),
            intended_wide.len().try_into().unwrap_or(i32::MAX),
            1,
        )
    };
    comparison == windows_sys::Win32::Globalization::CSTR_EQUAL
}

#[cfg(windows)]
fn normalize_windows_proof_path(path: &str) -> Result<String> {
    let path = path.replace('/', "\\");
    let path = if path
        .get(..8)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case(r"\\?\UNC\"))
    {
        format!(r"\\{}", &path[8..])
    } else if path
        .get(..4)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case(r"\\?\"))
    {
        path[4..].to_owned()
    } else {
        path
    };
    let unc = path.starts_with(r"\\");
    let components = path
        .split('\\')
        .filter(|component| !component.is_empty())
        .collect::<Vec<_>>();
    if components
        .iter()
        .any(|component| matches!(*component, "." | ".."))
    {
        bail!("Windows font proof path contains traversal components");
    }
    if unc {
        if components.len() < 3 {
            bail!("Windows UNC font proof path is incomplete");
        }
        return Ok(format!(r"\\{}", components.join("\\")));
    }
    if components.first().is_none_or(|drive| {
        let bytes = drive.as_bytes();
        bytes.len() != 2 || bytes[1] != b':' || !bytes[0].is_ascii_alphabetic()
    }) || components.len() < 2
    {
        bail!("Windows font proof path is not an absolute DOS or UNC path");
    }
    Ok(components.join("\\"))
}

#[cfg(windows)]
fn reject_windows_reparse_components(path: &Path) -> Result<()> {
    use std::os::windows::fs::MetadataExt;

    let mut current = PathBuf::new();
    for component in path.components() {
        current.push(component.as_os_str());
        if matches!(
            component,
            std::path::Component::Prefix(_) | std::path::Component::RootDir
        ) {
            continue;
        }
        let metadata = fs::symlink_metadata(&current)?;
        if metadata.file_attributes()
            & windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT
            != 0
        {
            bail!("refusing reparse-point font path {}", current.display());
        }
    }
    Ok(())
}

#[cfg(not(any(unix, windows)))]
fn open_no_follow(_path: &Path) -> Result<File> {
    bail!("secure font source reads are unsupported on this platform")
}

fn stable_file(before: &Metadata, after: &Metadata) -> bool {
    same_file(before, after)
        && before.len() == after.len()
        && before.modified().ok() == after.modified().ok()
}

#[cfg(unix)]
fn same_file(left: &Metadata, right: &Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;
    left.dev() == right.dev() && left.ino() == right.ino()
}

#[cfg(windows)]
fn same_file(left: &Metadata, right: &Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    left.volume_serial_number().is_some()
        && left.volume_serial_number() == right.volume_serial_number()
        && left.file_index().is_some()
        && left.file_index() == right.file_index()
}

#[cfg(not(any(unix, windows)))]
fn same_file(_left: &Metadata, _right: &Metadata) -> bool {
    false
}

fn require_editable_embedding(face: &Face<'_>) -> Result<()> {
    if !permissions_allow_editable_embedding(face.permissions()) {
        bail!(
            "OpenType embedding metadata does not allow portable editable embedding (preview/print-only fonts are unsupported); verify the font license separately"
        );
    }
    if !face.is_outline_embedding_allowed() {
        bail!(
            "OpenType embedding metadata does not allow editable outline embedding; verify the font license separately"
        );
    }
    Ok(())
}

fn permissions_allow_editable_embedding(permissions: Option<Permissions>) -> bool {
    matches!(
        permissions,
        Some(Permissions::Installable | Permissions::Editable)
    )
}

fn font_name(face: &Face<'_>, ids: &[u16]) -> Option<String> {
    ids.iter().find_map(|wanted| {
        face.names()
            .into_iter()
            .filter(|name| name.name_id == *wanted)
            .find_map(|name| name.to_string())
            .filter(|name| !name.trim().is_empty())
    })
}

fn sha256_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let digest = Sha256::digest(bytes);
    let mut encoded = String::with_capacity(digest.len() * 2);
    for byte in digest {
        encoded.push(HEX[usize::from(byte >> 4)] as char);
        encoded.push(HEX[usize::from(byte & 0x0f)] as char);
    }
    encoded
}

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    #[test]
    fn portable_editable_embedding_requires_installable_or_editable_permission() {
        assert!(permissions_allow_editable_embedding(Some(
            Permissions::Installable
        )));
        assert!(permissions_allow_editable_embedding(Some(
            Permissions::Editable
        )));
        assert!(!permissions_allow_editable_embedding(Some(
            Permissions::PreviewAndPrint
        )));
        assert!(!permissions_allow_editable_embedding(Some(
            Permissions::Restricted
        )));
        assert!(!permissions_allow_editable_embedding(None));
    }

    #[test]
    fn content_hash_is_lowercase_sha256_hex() {
        let hash = sha256_hex(b"Spectrum typography");
        assert_eq!(hash.len(), 64);
        assert!(hash.bytes().all(|byte| byte.is_ascii_hexdigit()));
        assert_eq!(hash, hash.to_ascii_lowercase());
        assert_eq!(
            hash,
            "08bf2a874548a4cad5f52023922a20f1b4b8724372b71a296673e9b6f1ce6696"
        );
    }

    #[test]
    fn file_identity_rejects_same_length_path_replacement() {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = fs::canonicalize(std::env::temp_dir())
            .unwrap_or_else(|_| std::env::temp_dir())
            .join(format!("prism-font-identity-{stamp}"));
        fs::create_dir_all(&directory).unwrap();
        let first = directory.join("first.ttf");
        let replacement = directory.join("replacement.ttf");
        fs::write(&first, b"same length").unwrap();
        fs::write(&replacement, b"same length").unwrap();
        let opened = File::open(&first).unwrap().metadata().unwrap();
        fs::remove_file(&first).unwrap();
        fs::rename(&replacement, &first).unwrap();
        let current = fs::metadata(&first).unwrap();
        assert!(!same_file(&opened, &current));
        fs::remove_dir_all(directory).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn final_handle_path_proof_rejects_a_different_intended_path() {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = fs::canonicalize(std::env::temp_dir())
            .unwrap_or_else(|_| std::env::temp_dir())
            .join(format!("prism-font-final-path-{stamp}"));
        fs::create_dir_all(&directory).unwrap();
        let opened_path = directory.join("opened.ttf");
        let intended_path = directory.join("intended.ttf");
        fs::write(&opened_path, b"same length").unwrap();
        fs::write(&intended_path, b"same length").unwrap();
        let opened = File::open(opened_path).unwrap();

        let error = require_windows_final_path_matches(&intended_path, &opened).unwrap_err();

        assert!(error.to_string().contains("does not match intended path"));
        fs::remove_dir_all(directory).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn windows_path_proof_normalizes_prefixes_separators_and_unicode_case_symmetrically() {
        assert!(
            windows_proof_paths_match(r"C:\Fonts\Source.ttf", r"\\?\C:\Fonts\Source.ttf").unwrap()
        );
        assert!(
            windows_proof_paths_match(r"\\?\C:\Fonts\Source.ttf", r"C:\Fonts\Source.ttf").unwrap()
        );
        assert!(
            windows_proof_paths_match(
                r"\\server\share\Fonts\Source.ttf",
                r"\\?\UNC\server\share\Fonts\Source.ttf"
            )
            .unwrap()
        );
        assert!(
            windows_proof_paths_match(
                r"\\?\UNC\server\share\Fonts\Source.ttf",
                r"//SERVER/share/fonts/source.TTF"
            )
            .unwrap()
        );
        assert!(
            windows_proof_paths_match(r"C:/École/Police.ttf", r"\\?\c:\éCOLE\POLICE.TTF").unwrap()
        );
        assert!(
            !windows_proof_paths_match(
                r"C:\PROGRA~1\Fonts\Source.ttf",
                r"\\?\C:\Program Files\Fonts\Source.ttf"
            )
            .unwrap()
        );
    }

    #[cfg(windows)]
    #[test]
    fn final_handle_proof_rejects_an_ancestor_junction_swapped_after_scan() {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = fs::canonicalize(std::env::temp_dir())
            .unwrap_or_else(|_| std::env::temp_dir())
            .join(format!("prism-font-junction-swap-{stamp}"));
        let intended_directory = directory.join("intended");
        let saved_directory = directory.join("saved-intended");
        let redirect_directory = directory.join("redirect");
        fs::create_dir_all(&intended_directory).unwrap();
        fs::create_dir_all(&redirect_directory).unwrap();
        fs::write(intended_directory.join("font.ttf"), b"same length").unwrap();
        fs::write(redirect_directory.join("font.ttf"), b"same length").unwrap();
        let intended_for_hook = intended_directory.clone();
        let saved_for_hook = saved_directory.clone();
        let redirect_for_hook = redirect_directory.clone();
        WINDOWS_AFTER_SCAN_HOOK.with(|hook| {
            hook.replace(Some(Box::new(move || {
                fs::rename(&intended_for_hook, &saved_for_hook).unwrap();
                let status = std::process::Command::new("cmd")
                    .args(["/C", "mklink", "/J"])
                    .arg(&intended_for_hook)
                    .arg(&redirect_for_hook)
                    .status()
                    .unwrap();
                assert!(status.success(), "test junction should be creatable");
            })));
        });

        let error = open_no_follow(&intended_directory.join("font.ttf")).unwrap_err();

        assert!(error.to_string().contains("does not match intended path"));
        fs::remove_dir(&intended_directory).unwrap();
        fs::remove_dir_all(directory).unwrap();
    }
}
