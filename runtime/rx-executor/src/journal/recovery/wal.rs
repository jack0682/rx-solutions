//! Strict validation of already-copied SQLite WAL bytes before SQLite can ignore them.
//!
//! Based on SQLite file format 4.1/4.2 and the pinned libsqlite3-sys 0.38.2 wal.c:
//! WAL header fields/checksums are big-endian; checksum input word order follows magic.
//! Frame checksum input is its FIRST EIGHT header bytes followed by page data, chained
//! from the validated WAL header and previous frames. Salt bytes are checked separately.
//!
//! Absent/zero WAL supplies no frames. A valid 32-byte header also supplies no frames;
//! accepting that shape does not prove a historical committed head. Nonempty frame WAL
//! must contain only complete, same-salt, checksum-valid frames and end with a commit.
//! SQLite permits reused old-salt tails, partial writes, and uncommitted spill frames.
//! This bounded inspector rejects those as WAL_TAIL_UNPROVEN rather than guessing which
//! tail is disposable. It never silently falls back to the main DB after invalid bytes.
//! Whole-WAL deletion/truncation to an otherwise valid earlier state, and coordinated
//! rollback of DB/WAL, cannot be detected without an independent trusted committed head.

use rx_ports::{Result, StoreError};
use std::{
    fs::{self, File},
    io::Read,
    path::Path,
};

const MAX_BYTES: u64 = 64 * 1024 * 1024;
const MAX_PAGE: u32 = 0xffff_fffe;
const MAGIC_LE: u32 = 0x377f_0682;
const MAGIC_BE: u32 = 0x377f_0683;
const WAL_VERSION: u32 = 3_007_000;

fn invalid(message: &str) -> StoreError {
    StoreError::Integrity(format!("RECOVERY_WAL_INVALID: {message}"))
}
fn tail(message: &str) -> StoreError {
    StoreError::Integrity(format!("RECOVERY_WAL_TAIL_UNPROVEN: {message}"))
}
fn read_error(error: std::io::Error) -> StoreError {
    StoreError::Integrity(format!("RECOVERY_WAL_UNREADABLE: {error}"))
}
fn be(bytes: &[u8]) -> u32 {
    u32::from_be_bytes(bytes.try_into().expect("fixed four-byte format field"))
}
fn page_size(value: u32) -> bool {
    (512..=65_536).contains(&value) && value.is_power_of_two()
}
fn checksum(bytes: &[u8], big_endian: bool, state: &mut [u32; 2]) {
    debug_assert_eq!(bytes.len() % 8, 0);
    for pair in bytes.as_chunks::<8>().0 {
        let word = |bytes: &[u8]| {
            let bytes = bytes.try_into().expect("fixed checksum word");
            if big_endian {
                u32::from_be_bytes(bytes)
            } else {
                u32::from_le_bytes(bytes)
            }
        };
        state[0] = state[0]
            .wrapping_add(word(&pair[..4]))
            .wrapping_add(state[1]);
        state[1] = state[1]
            .wrapping_add(word(&pair[4..]))
            .wrapping_add(state[0]);
    }
}
fn regular_file(path: &Path) -> Result<(File, u64)> {
    let metadata = fs::symlink_metadata(path).map_err(read_error)?;
    if !metadata.file_type().is_file() {
        return Err(invalid("copied source must be a regular file"));
    }
    if metadata.len() > MAX_BYTES {
        return Err(StoreError::Invalid(
            "RECOVERY_INSPECT_LIMIT_EXCEEDED: WAL/DB exceeds 64 MiB".into(),
        ));
    }
    let file = File::open(path).map_err(read_error)?;
    if file.metadata().map_err(read_error)?.len() != metadata.len() {
        return Err(invalid("copied source size changed"));
    }
    Ok((file, metadata.len()))
}

/// Only the private temporary copies are supplied here. No SQLite or original file is opened.
pub(super) fn validate_copy(database: &Path) -> Result<()> {
    let (mut db, db_size) = regular_file(database)?;
    let mut db_header = [0_u8; 100];
    db.read_exact(&mut db_header).map_err(read_error)?;
    if &db_header[..16] != b"SQLite format 3\0" {
        return Err(invalid("database header magic differs"));
    }
    let raw_page = u16::from_be_bytes([db_header[16], db_header[17]]);
    let db_page = if raw_page == 1 {
        65_536
    } else {
        u32::from(raw_page)
    };
    if !page_size(db_page) || db_size < u64::from(db_page) || db_size % u64::from(db_page) != 0 {
        return Err(invalid(
            "database page size or complete page coverage differs",
        ));
    }
    let mut wal_name = database.as_os_str().to_os_string();
    wal_name.push("-wal");
    let path = Path::new(&wal_name);
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(read_error(error)),
        Ok(_) => {}
    }
    let (mut wal, size) = regular_file(path)?;
    if size == 0 {
        return Ok(());
    }
    if size < 32 {
        return Err(tail("incomplete WAL header"));
    }
    let mut header = [0_u8; 32];
    wal.read_exact(&mut header).map_err(read_error)?;
    let big_endian = match be(&header[..4]) {
        MAGIC_LE => false,
        MAGIC_BE => true,
        _ => return Err(invalid("WAL header magic differs")),
    };
    if be(&header[4..8]) != WAL_VERSION {
        return Err(invalid("unsupported WAL format version"));
    }
    let page = be(&header[8..12]);
    if !page_size(page) || page != db_page {
        return Err(invalid("WAL/database page sizes differ"));
    }
    let mut state = [0_u32; 2];
    checksum(&header[..24], big_endian, &mut state);
    if state != [be(&header[24..28]), be(&header[28..32])] {
        return Err(invalid("WAL header checksum differs"));
    }
    if size == 32 {
        return Ok(());
    }
    let frame_size = u64::from(page) + 24;
    if (size - 32) % frame_size != 0 {
        return Err(tail("partial frame or residual trailing bytes"));
    }
    let frames = (size - 32) / frame_size;
    let mut last_commit = false;
    let mut frame_header = [0_u8; 24];
    let mut buffer = [0_u8; 4096];
    for _ in 0..frames {
        wal.read_exact(&mut frame_header).map_err(read_error)?;
        if frame_header[8..16] != header[16..24] {
            return Err(tail(
                "frame salt differs; reuse leftovers or damage are not distinguished",
            ));
        }
        let page_number = be(&frame_header[..4]);
        let commit_pages = be(&frame_header[4..8]);
        if page_number == 0 || page_number > MAX_PAGE || commit_pages > MAX_PAGE {
            return Err(invalid(
                "frame page number or commit database size is outside SQLite range",
            ));
        }
        // A commit is a nonzero database-size field. It is not an arbitrary boolean.
        // Do not require page_number <= commit_pages: a shrink can discard spill pages.
        last_commit = commit_pages != 0;
        checksum(&frame_header[..8], big_endian, &mut state);
        let mut remaining = page as usize;
        while remaining > 0 {
            let count = remaining.min(buffer.len());
            wal.read_exact(&mut buffer[..count]).map_err(read_error)?;
            checksum(&buffer[..count], big_endian, &mut state);
            remaining -= count;
        }
        if state != [be(&frame_header[16..20]), be(&frame_header[20..24])] {
            return Err(tail(
                "frame checksum differs; a committed prefix is not enough to accept an uncertain tail",
            ));
        }
    }
    if !last_commit {
        return Err(tail("complete but uncommitted final frames"));
    }
    let mut extra = [0_u8; 1];
    if wal.read(&mut extra).map_err(read_error)? != 0
        || wal.metadata().map_err(read_error)?.len() != size
    {
        return Err(tail("copied WAL changed during validation"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs::OpenOptions, path::PathBuf};

    // Synthetic byte-format tests only. Real SQLite transaction/WAL fixtures are separate.
    fn fixture(page: u32) -> (tempfile::TempDir, PathBuf, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("journal.sqlite3");
        let wal = dir.path().join("journal.sqlite3-wal");
        let mut header = vec![0_u8; page as usize];
        header[..16].copy_from_slice(b"SQLite format 3\0");
        let field = if page == 65_536 { 1_u16 } else { page as u16 };
        header[16..18].copy_from_slice(&field.to_be_bytes());
        fs::write(&db, header).unwrap();
        (dir, db, wal)
    }
    fn reference_checksum(bytes: &[u8], big: bool, mut sums: [u32; 2]) -> [u32; 2] {
        let words: Vec<u32> = bytes
            .as_chunks::<4>()
            .0
            .iter()
            .map(|w| {
                let a: [u8; 4] = *w;
                if big {
                    u32::from_be_bytes(a)
                } else {
                    u32::from_le_bytes(a)
                }
            })
            .collect();
        for pair in words.as_chunks::<2>().0 {
            sums[0] = (u64::from(sums[0]) + u64::from(pair[0]) + u64::from(sums[1])) as u32;
            sums[1] = (u64::from(sums[1]) + u64::from(pair[1]) + u64::from(sums[0])) as u32;
        }
        sums
    }
    fn wal_bytes(page: u32, big: bool, frames: &[(u32, u32)]) -> Vec<u8> {
        let mut result = vec![];
        for word in [
            if big { MAGIC_BE } else { MAGIC_LE },
            WAL_VERSION,
            page,
            0,
            7,
            11,
        ] {
            result.extend(word.to_be_bytes());
        }
        let mut sums = reference_checksum(&result, big, [0, 0]);
        result.extend(sums[0].to_be_bytes());
        result.extend(sums[1].to_be_bytes());
        for (index, &(number, commit)) in frames.iter().enumerate() {
            let mut frame = vec![];
            for word in [number, commit, 7, 11] {
                frame.extend(word.to_be_bytes());
            }
            let data = vec![index as u8 + 3; page as usize];
            sums = reference_checksum(&frame[..8], big, sums);
            sums = reference_checksum(&data, big, sums);
            frame.extend(sums[0].to_be_bytes());
            frame.extend(sums[1].to_be_bytes());
            frame.extend(data);
            result.extend(frame);
        }
        result
    }
    #[test]
    fn checksum_word_order_and_accumulation_have_independent_known_values() {
        for big in [false, true] {
            let mut bytes = vec![];
            for word in [1_u32, 2, 3, 4] {
                bytes.extend(if big {
                    word.to_be_bytes()
                } else {
                    word.to_le_bytes()
                });
            }
            let mut sums = [0, 0];
            checksum(&bytes, big, &mut sums);
            assert_eq!(sums, [7, 14]);
            bytes.clear();
            for word in [5_u32, 6] {
                bytes.extend(if big {
                    word.to_be_bytes()
                } else {
                    word.to_le_bytes()
                });
            }
            checksum(&bytes, big, &mut sums);
            assert_eq!(sums, [26, 46]);
        }
    }
    #[test]
    fn absent_empty_header_only_and_complete_commits_are_distinct_valid_shapes() {
        for (page, big) in [(512, false), (4096, true), (65_536, false), (65_536, true)] {
            let (_dir, db, wal) = fixture(page);
            validate_copy(&db).unwrap();
            fs::write(&wal, []).unwrap();
            validate_copy(&db).unwrap();
            fs::write(&wal, wal_bytes(page, big, &[])).unwrap();
            validate_copy(&db).unwrap();
            // Repeated final commit frames are permitted sector padding, not corruption.
            fs::write(&wal, wal_bytes(page, big, &[(1, 0), (2, 2), (2, 2)])).unwrap();
            validate_copy(&db).unwrap();
        }
    }
    #[test]
    fn header_fields_and_header_checksum_cannot_silently_drop_the_wal() {
        let (_dir, db, wal) = fixture(512);
        for offset in [0, 4, 8, 16, 24, 28] {
            let mut bytes = wal_bytes(512, false, &[(1, 1)]);
            bytes[offset] ^= 1;
            fs::write(&wal, bytes).unwrap();
            assert!(validate_copy(&db).is_err(), "header byte {offset}");
        }
        fs::write(&wal, wal_bytes(1024, false, &[(1, 1)])).unwrap();
        assert!(validate_copy(&db).is_err());
    }
    #[test]
    fn page_data_checksums_salts_partial_and_uncommitted_tails_are_rejected() {
        let (_dir, db, wal) = fixture(512);
        for offset in [32 + 8, 32 + 16, 32 + 24 + 5] {
            let mut bytes = wal_bytes(512, false, &[(1, 1)]);
            bytes[offset] ^= 1;
            fs::write(&wal, bytes).unwrap();
            assert!(
                validate_copy(&db)
                    .unwrap_err()
                    .to_string()
                    .contains("WAL_TAIL_UNPROVEN")
            );
        }
        let mut partial = wal_bytes(512, false, &[(1, 1)]);
        partial.extend([0_u8; 12]);
        fs::write(&wal, partial).unwrap();
        assert!(
            validate_copy(&db)
                .unwrap_err()
                .to_string()
                .contains("WAL_TAIL_UNPROVEN")
        );
        for frames in [vec![(1, 0)], vec![(1, 1), (2, 0)]] {
            fs::write(&wal, wal_bytes(512, true, &frames)).unwrap();
            assert!(
                validate_copy(&db)
                    .unwrap_err()
                    .to_string()
                    .contains("WAL_TAIL_UNPROVEN")
            );
        }
        let mut reused = wal_bytes(512, true, &[(1, 1), (2, 2)]);
        reused[32 + 536 + 8..32 + 536 + 12].copy_from_slice(&6_u32.to_be_bytes());
        fs::write(&wal, reused).unwrap();
        assert!(
            validate_copy(&db)
                .unwrap_err()
                .to_string()
                .contains("WAL_TAIL_UNPROVEN")
        );
    }
    #[test]
    fn invalid_page_numbers_commit_sizes_and_oversize_files_fail_before_unbounded_reads() {
        let (_dir, db, wal) = fixture(512);
        for frame in [(0, 1), (u32::MAX, 1), (1, u32::MAX)] {
            fs::write(&wal, wal_bytes(512, false, &[frame])).unwrap();
            assert!(validate_copy(&db).is_err());
        }
        OpenOptions::new()
            .write(true)
            .open(&wal)
            .unwrap()
            .set_len(MAX_BYTES + 1)
            .unwrap();
        assert!(
            validate_copy(&db)
                .unwrap_err()
                .to_string()
                .contains("LIMIT_EXCEEDED")
        );
    }
}
