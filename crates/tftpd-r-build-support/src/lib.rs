//! Build-time helpers for the TftpdR workspace.
//!
//! This crate is only used from `build.rs` files; nothing here is linked into a
//! shipped binary. It currently embeds the product icon in the Windows
//! executables as an `RT_GROUP_ICON`/`RT_ICON` PE resource.
//!
//! The `.res` container is generated in Rust from the committed `.ico` file, so
//! no generated binary is checked in. MSVC's `link.exe` consumes the `.res`
//! directly. GNU targets have no such linker input, so `windres` converts the
//! same `.res` to a COFF object with an `.rsrc` section. The output is
//! byte-for-byte the same resource layout that `windres` itself writes for
//! `1 ICON "app-icon.ico"` (verified against binutils 2.46.1); keeping the
//! generator well-formed is covered by the unit tests in this file.

use std::env;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Size of a `.res` resource header with ordinal type and name.
const RESOURCE_HEADER_BYTES: usize = 32;
/// `RT_ICON` resource type.
const RT_ICON: u16 = 3;
/// `RT_GROUP_ICON` resource type.
const RT_GROUP_ICON: u16 = 14;
/// Icon-group resource ID; a single product icon per executable is enough.
const ICON_GROUP_ID: u16 = 1;
/// `MAKELANGID(LANG_ENGLISH, SUBLANG_ENGLISH_US)`, the usual resource language.
const LANGUAGE_EN_US: u16 = 0x0409;
/// `MOVEABLE | DISCARDABLE`, the memory flags `windres` emits for icons.
const MEMORY_FLAGS: u16 = 0x1010;
/// Upper bound on ICO frames; the product icon has seven.
const MAX_ICON_IMAGES: usize = 64;
/// Fixed ICO header: reserved, type, count.
const ICO_HEADER_BYTES: usize = 6;
/// One ICO directory entry.
const ICO_ENTRY_BYTES: usize = 16;

/// A build-time failure with a message suitable for a `build.rs` panic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error {
    message: String,
}

impl Error {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for Error {}

/// Embeds `icon` as the icon of every binary target of the calling package.
///
/// `icon` is resolved relative to the calling package's `CARGO_MANIFEST_DIR`.
/// The function is a no-op unless the build target is Windows. On MSVC targets
/// the generated `.res` file is passed to `link.exe`; on GNU targets `windres`
/// (from `PATH`, or `WINDRES`) converts it to a COFF object first.
///
/// Emits `rerun-if-changed` for the icon, so replacing the artwork is enough to
/// rebuild the resource.
pub fn embed_windows_icon(icon: impl AsRef<Path>) -> Result<(), Error> {
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return Ok(());
    }

    let icon = resolve_manifest_path(icon.as_ref())?;
    println!("cargo:rerun-if-changed={}", icon.display());
    let bytes = fs::read(&icon).map_err(|error| {
        Error::new(format!(
            "cannot read the icon at {}: {error}",
            icon.display()
        ))
    })?;
    let images = parse_ico(&bytes)
        .map_err(|error| Error::new(format!("invalid icon {}: {error}", icon.display())))?;
    let resource = build_resource(&images);

    let out_dir = env::var_os("OUT_DIR")
        .map(PathBuf::from)
        .ok_or_else(|| Error::new("OUT_DIR is not set; call this from a Cargo build script"))?;
    let resource_path = out_dir.join("tftpd-r-icon.res");
    fs::write(&resource_path, &resource).map_err(|error| {
        Error::new(format!("cannot write {}: {error}", resource_path.display()))
    })?;

    let link_input = if env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
        resource_path
    } else {
        let object_path = out_dir.join("tftpd-r-icon.o");
        compile_resource_with_windres(&resource_path, &object_path)?;
        object_path
    };
    println!("cargo:rustc-link-arg-bins={}", link_input.display());
    Ok(())
}

fn resolve_manifest_path(path: &Path) -> Result<PathBuf, Error> {
    let manifest_dir = env::var_os("CARGO_MANIFEST_DIR")
        .map(PathBuf::from)
        .ok_or_else(|| {
            Error::new("CARGO_MANIFEST_DIR is not set; call this from a Cargo build script")
        })?;
    // `Path::join` keeps an absolute `path` unchanged.
    Ok(manifest_dir.join(path))
}

fn compile_resource_with_windres(resource: &Path, object: &Path) -> Result<(), Error> {
    let windres = env::var_os("WINDRES").unwrap_or_else(|| "windres".into());
    let mut command = Command::new(&windres);
    command
        .arg("--input-format=res")
        .arg("--output-format=coff");
    if let Some(target) = windres_target(&env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default()) {
        command.arg(format!("--target={target}"));
    }
    command.arg("-o").arg(object).arg(resource);

    let output = command.output().map_err(|error| {
        Error::new(format!(
            "cannot run windres ({}): {error}. Install MinGW-w64 binutils or set WINDRES",
            windres.to_string_lossy()
        ))
    })?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(Error::new(format!(
            "windres failed for {} with {}: {}",
            resource.display(),
            output.status,
            stderr.trim()
        )));
    }
    Ok(())
}

/// BFD target for the host's `windres`, when the default would be wrong (i.e.
/// an x86 build on an x86_64 toolchain). Native arm64 GNU builds keep the
/// toolchain default.
fn windres_target(arch: &str) -> Option<&'static str> {
    match arch {
        "x86_64" => Some("pe-x86-64"),
        "x86" => Some("pe-i386"),
        _ => None,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct IconImage {
    width: u8,
    height: u8,
    color_count: u8,
    planes: u16,
    bit_count: u16,
    data: Vec<u8>,
}

fn parse_ico(bytes: &[u8]) -> Result<Vec<IconImage>, Error> {
    if bytes.len() < ICO_HEADER_BYTES {
        return Err(Error::new("the ICO header is truncated"));
    }
    let reserved = read_u16(bytes, 0);
    let kind = read_u16(bytes, 2);
    let count = usize::from(read_u16(bytes, 4));
    if reserved != 0 {
        return Err(Error::new("the ICO reserved field is not zero"));
    }
    if kind != 1 {
        return Err(Error::new(format!(
            "the ICO type is {kind}, expected 1 (icon)"
        )));
    }
    if count == 0 {
        return Err(Error::new("the ICO contains no images"));
    }
    if count > MAX_ICON_IMAGES {
        return Err(Error::new(format!(
            "the ICO contains {count} images, more than the {MAX_ICON_IMAGES} supported"
        )));
    }
    let directory_end = ICO_HEADER_BYTES + count * ICO_ENTRY_BYTES;
    if bytes.len() < directory_end {
        return Err(Error::new("the ICO directory is truncated"));
    }

    let mut images = Vec::with_capacity(count);
    for index in 0..count {
        let start = ICO_HEADER_BYTES + index * ICO_ENTRY_BYTES;
        let entry = &bytes[start..start + ICO_ENTRY_BYTES];
        if entry[3] != 0 {
            return Err(Error::new(format!(
                "ICO image {index} has a non-zero reserved byte"
            )));
        }
        let size = u32::from_le_bytes([entry[8], entry[9], entry[10], entry[11]]) as usize;
        let offset = u32::from_le_bytes([entry[12], entry[13], entry[14], entry[15]]) as usize;
        if size == 0 {
            return Err(Error::new(format!("ICO image {index} is empty")));
        }
        let end = offset
            .checked_add(size)
            .filter(|end| *end <= bytes.len())
            .ok_or_else(|| {
                Error::new(format!(
                    "ICO image {index} points outside the file (offset {offset}, size {size})"
                ))
            })?;
        if offset < directory_end {
            return Err(Error::new(format!(
                "ICO image {index} overlaps the directory"
            )));
        }
        images.push(IconImage {
            width: entry[0],
            height: entry[1],
            color_count: entry[2],
            planes: read_u16(entry, 4),
            bit_count: read_u16(entry, 6),
            data: bytes[offset..end].to_vec(),
        });
    }
    Ok(images)
}

/// Builds a `.res` file with one null resource, one `RT_ICON` per ICO frame and
/// a trailing `RT_GROUP_ICON` that references them.
fn build_resource(images: &[IconImage]) -> Vec<u8> {
    let mut resource = Vec::new();
    // A `.res` file starts with a null resource header.
    push_resource_header(&mut resource, 0, 0, 0, 0, 0);

    for (index, image) in images.iter().enumerate() {
        push_resource_header(
            &mut resource,
            RT_ICON,
            icon_id(index),
            image.data.len() as u32,
            LANGUAGE_EN_US,
            MEMORY_FLAGS,
        );
        resource.extend_from_slice(&image.data);
        pad_to_dword(&mut resource);
    }

    let mut group = Vec::with_capacity(6 + images.len() * 14);
    push_u16(&mut group, 0); // reserved
    push_u16(&mut group, 1); // resource type: icon
    push_u16(&mut group, images.len() as u16);
    for (index, image) in images.iter().enumerate() {
        group.push(image.width);
        group.push(image.height);
        group.push(image.color_count);
        group.push(0); // reserved
        push_u16(&mut group, image.planes);
        push_u16(&mut group, image.bit_count);
        push_u32(&mut group, image.data.len() as u32);
        push_u16(&mut group, icon_id(index));
    }
    push_resource_header(
        &mut resource,
        RT_GROUP_ICON,
        ICON_GROUP_ID,
        group.len() as u32,
        LANGUAGE_EN_US,
        MEMORY_FLAGS,
    );
    resource.extend_from_slice(&group);
    pad_to_dword(&mut resource);
    resource
}

fn icon_id(index: usize) -> u16 {
    (index + 1) as u16
}

fn push_resource_header(
    out: &mut Vec<u8>,
    type_id: u16,
    name_id: u16,
    data_size: u32,
    language: u16,
    memory_flags: u16,
) {
    push_u32(out, data_size);
    push_u32(out, RESOURCE_HEADER_BYTES as u32);
    push_u16(out, 0xFFFF); // ordinal type follows
    push_u16(out, type_id);
    push_u16(out, 0xFFFF); // ordinal name follows
    push_u16(out, name_id);
    push_u32(out, 0); // data version
    push_u16(out, memory_flags);
    push_u16(out, language);
    push_u32(out, 0); // version
    push_u32(out, 0); // characteristics
}

fn pad_to_dword(out: &mut Vec<u8>) {
    while !out.len().is_multiple_of(4) {
        out.push(0);
    }
}

fn push_u16(out: &mut Vec<u8>, value: u16) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn push_u32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn read_u16(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes([bytes[offset], bytes[offset + 1]])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn product_icon() -> Vec<u8> {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../tftpd-r-gui-slint/ui/icons/app-icon.ico");
        fs::read(&path).unwrap_or_else(|error| panic!("read {}: {error}", path.display()))
    }

    #[test]
    fn parses_the_product_icon_frames() {
        let bytes = product_icon();
        let images = parse_ico(&bytes).expect("parse the product icon");
        let dimensions: Vec<(u8, u8)> = images
            .iter()
            .map(|image| (image.width, image.height))
            .collect();
        assert_eq!(
            dimensions,
            vec![
                (16, 16),
                (24, 24),
                (32, 32),
                (48, 48),
                (64, 64),
                (128, 128),
                (0, 0)
            ]
        );
        let sizes: Vec<usize> = images.iter().map(|image| image.data.len()).collect();
        assert_eq!(sizes, vec![1128, 2440, 4264, 9640, 16936, 67624, 9944]);
        assert_eq!(read_u16(&images[0].data, 0), 0x0028); // BITMAPINFOHEADER
        assert_eq!(&images[6].data[..8], b"\x89PNG\r\n\x1a\n");
    }

    fn single_frame_ico(size: u32, offset: u32, image: &[u8]) -> Vec<u8> {
        let mut bytes = vec![0, 0, 1, 0, 1, 0]; // reserved, icon, one entry
        bytes.extend_from_slice(&[16, 16, 0, 0]); // width, height, colours, reserved
        bytes.extend_from_slice(&1u16.to_le_bytes()); // planes
        bytes.extend_from_slice(&32u16.to_le_bytes()); // bit count
        bytes.extend_from_slice(&size.to_le_bytes());
        bytes.extend_from_slice(&offset.to_le_bytes());
        bytes.extend_from_slice(image);
        bytes
    }

    #[test]
    fn rejects_malformed_icons() {
        assert!(parse_ico(&[]).is_err());
        assert!(parse_ico(&[1, 0, 1, 0, 1, 0]).is_err()); // reserved != 0
        assert!(parse_ico(&[0, 0, 2, 0, 0, 0]).is_err()); // not an icon
        assert!(parse_ico(&[0, 0, 1, 0, 0, 0]).is_err()); // zero images

        assert!(parse_ico(&single_frame_ico(4, 22, &[])).is_err()); // past EOF
        assert!(parse_ico(&single_frame_ico(0, 22, &[])).is_err()); // empty frame
        assert!(parse_ico(&single_frame_ico(1, 6, &[0])).is_err()); // in directory
    }

    #[test]
    fn resource_contains_a_group_and_every_frame() {
        let images = parse_ico(&product_icon()).expect("parse the product icon");
        let resource = build_resource(&images);
        assert!(resource.len().is_multiple_of(4));
        let entries = parse_test_resource(&resource);

        assert_eq!(entries.len(), images.len() + 2);
        assert_eq!(entries[0].type_id, 0);
        assert_eq!(entries[0].name_id, 0);
        assert!(entries[0].data.is_empty());

        for (index, image) in images.iter().enumerate() {
            let entry = &entries[index + 1];
            assert_eq!(entry.type_id, RT_ICON);
            assert_eq!(entry.name_id, icon_id(index));
            assert_eq!(entry.language, LANGUAGE_EN_US);
            assert_eq!(entry.data, image.data);
        }

        let group = entries.last().expect("group entry");
        assert_eq!(group.type_id, RT_GROUP_ICON);
        assert_eq!(group.name_id, ICON_GROUP_ID);
        assert_eq!(group.data.len(), 6 + 14 * images.len());
        assert_eq!(read_u16(&group.data, 0), 0);
        assert_eq!(read_u16(&group.data, 2), 1);
        assert_eq!(usize::from(read_u16(&group.data, 4)), images.len());
        for (index, image) in images.iter().enumerate() {
            let start = 6 + 14 * index;
            let entry = &group.data[start..start + 14];
            assert_eq!(entry[0], image.width);
            assert_eq!(entry[1], image.height);
            assert_eq!(entry[2], image.color_count);
            assert_eq!(entry[3], 0);
            assert_eq!(read_u16(entry, 4), image.planes);
            assert_eq!(read_u16(entry, 6), image.bit_count);
            assert_eq!(
                u32::from_le_bytes([entry[8], entry[9], entry[10], entry[11]]) as usize,
                image.data.len()
            );
            assert_eq!(read_u16(entry, 12), icon_id(index));
        }
    }

    struct TestResource {
        type_id: u16,
        name_id: u16,
        language: u16,
        data: Vec<u8>,
    }

    /// Minimal reader for the generator's own output. The real linker/PE
    /// consumer is exercised by the session verification, not by this test.
    fn parse_test_resource(bytes: &[u8]) -> Vec<TestResource> {
        let mut entries = Vec::new();
        let mut offset = 0;
        while offset < bytes.len() {
            assert!(offset + RESOURCE_HEADER_BYTES <= bytes.len());
            let data_size =
                u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap()) as usize;
            let header_size =
                u32::from_le_bytes(bytes[offset + 4..offset + 8].try_into().unwrap()) as usize;
            assert_eq!(header_size, RESOURCE_HEADER_BYTES);
            assert_eq!(read_u16(bytes, offset + 8), 0xFFFF);
            let type_id = read_u16(bytes, offset + 10);
            assert_eq!(read_u16(bytes, offset + 12), 0xFFFF);
            let name_id = read_u16(bytes, offset + 14);
            let language = read_u16(bytes, offset + 22);
            let start = offset + header_size;
            let end = start + data_size;
            assert!(end <= bytes.len());
            entries.push(TestResource {
                type_id,
                name_id,
                language,
                data: bytes[start..end].to_vec(),
            });
            offset = (end + 3) & !3;
        }
        entries
    }
}
