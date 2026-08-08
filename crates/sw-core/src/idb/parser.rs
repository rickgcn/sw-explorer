//! Line-oriented parser for `product.idb` files.
//!
//! The IDB is decoded as Latin-1 (one byte equals one character) so that
//! the raw line preserved in [`EntryOrigin`] matches the file exactly.
use crate::diagnostic::Diagnostic;
use crate::error::Result;
use crate::idb::attribute;
use crate::idb::{Entry, EntryId, EntryOrigin, FileType};
use crate::names::SubsystemName;
use crate::path::IrixPath;
use std::path::Path;

/// Parses a complete IDB file into entries.
///
/// Malformed lines are skipped with a diagnostic; the rest of the file is
/// still parsed. Entry payload locators are left `None` here and filled in
/// when the image layout is computed.
pub fn parse(
    bytes: &[u8],
    idb_path: &Path,
    diagnostics: &mut Vec<Diagnostic>,
) -> Result<Vec<Entry>> {
    // Latin-1 decoding: every byte maps to the code point of the same value.
    let text: String = bytes.iter().map(|&b| b as char).collect();
    let mut entries = Vec::new();

    for (index, raw_line) in text.lines().enumerate() {
        let line_number = index + 1;
        let line = raw_line.trim_end();
        if line.is_empty() {
            continue;
        }
        let origin = format!("{}:{line_number}", idb_path.display());

        let Some(entry) = parse_line(line, idb_path, line_number, raw_line, &origin, diagnostics)
        else {
            continue;
        };
        entries.push(entry);
    }

    // Assign stable ids after filtering, in IDB order.
    for (index, entry) in entries.iter_mut().enumerate() {
        entry.id = EntryId(index);
    }

    Ok(entries)
}

fn parse_line(
    line: &str,
    idb_path: &Path,
    line_number: usize,
    raw_line: &str,
    origin: &str,
    diagnostics: &mut Vec<Diagnostic>,
) -> Option<Entry> {
    // Six fixed fields: type mode owner group path source. The tail holds
    // the subsystem name and the attribute list in *either* order: newer
    // media put the subsystem first (`... eoe1.sw.unix sum(1) ...`), older
    // media append it (`... sum(62014) size(595) cmpsize(459) vfr.sw.vfr`).
    let mut fixed: Vec<&str> = Vec::with_capacity(6);
    let mut rest = line;
    for _ in 0..6 {
        rest = rest.trim_start();
        let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
        if end == 0 {
            break;
        }
        fixed.push(&rest[..end]);
        rest = &rest[end..];
    }
    if fixed.len() < 6 {
        diagnostics.push(Diagnostic::error(
            format!("truncated IDB line: {line:?}"),
            Some(origin.to_string()),
        ));
        return None;
    }

    let mut type_chars = fixed[0].chars();
    let letter = type_chars.next()?;
    let file_type = FileType::from_letter(letter);
    if let FileType::Unknown(_) = file_type {
        diagnostics.push(Diagnostic::warning(
            format!("unknown entry type letter {letter:?}"),
            Some(origin.to_string()),
        ));
    }

    let mode = match u32::from_str_radix(fixed[1], 8) {
        Ok(mode) => mode,
        Err(_) => {
            diagnostics.push(Diagnostic::error(
                format!("invalid octal mode {:?}", fixed[1]),
                Some(origin.to_string()),
            ));
            return None;
        }
    };

    let owner = fixed[2].to_string();
    let group = fixed[3].to_string();
    let raw_path = fixed[4].to_string();
    let source_path = fixed[5].to_string();

    // Tokenize the tail parenthesis- and quote-aware, then pick out the
    // one bare token shaped like a subsystem name. Real media always have
    // exactly one such token per entry line.
    let tokens = attribute::tokenize(rest.trim(), origin, diagnostics);
    let mut subsystem_index: Option<usize> = None;
    for (index, token) in tokens.iter().enumerate() {
        if token.has_args() {
            continue;
        }
        if SubsystemName::parse(&token.name).is_ok() {
            if subsystem_index.is_some() {
                diagnostics.push(Diagnostic::warning(
                    format!("multiple subsystem-shaped tokens on line; using the first: {line:?}"),
                    Some(origin.to_string()),
                ));
                break;
            }
            subsystem_index = Some(index);
        }
    }
    let Some(subsystem_index) = subsystem_index else {
        diagnostics.push(Diagnostic::error(
            format!("no subsystem name on IDB line: {line:?}"),
            Some(origin.to_string()),
        ));
        return None;
    };

    let path = match IrixPath::new(&raw_path) {
        Ok(path) => path,
        Err(error) => {
            diagnostics.push(Diagnostic::error(
                format!("skipping entry with unsafe path: {error}"),
                Some(origin.to_string()),
            ));
            return None;
        }
    };

    let subsystem = SubsystemName::parse(&tokens[subsystem_index].name).ok()?;
    let attributes = tokens
        .into_iter()
        .enumerate()
        .filter(|(index, _)| *index != subsystem_index)
        .map(|(_, token)| attribute::classify(token, origin, diagnostics))
        .collect();

    Some(Entry {
        id: EntryId(0),
        file_type,
        mode,
        owner,
        group,
        path,
        raw_path,
        source_path,
        subsystem,
        attributes,
        payload: None,
        origin: EntryOrigin {
            idb_path: idb_path.to_path_buf(),
            line_number,
            raw_line: raw_line.to_string(),
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::idb::ConfigMode;
    use crate::idb::attribute::IdbAttribute;

    fn parse_idb(text: &str) -> (Vec<Entry>, Vec<Diagnostic>) {
        let mut diagnostics = Vec::new();
        let entries = parse(text.as_bytes(), Path::new("test.idb"), &mut diagnostics).unwrap();
        (entries, diagnostics)
    }

    #[test]
    fn parses_real_line_shapes() {
        let (entries, diagnostics) = parse_idb(
            "d 0755 root sys . work/irix/. eoe1.sw.unix exitop(\"rm -fr $rbase/etc/xutmp\")\n\
             f 0755 root sys .bin.mv.sh work/irix/cmd/adm/bin.mv.sh eoe1.sw.unix sum(48641) size(1426) postop($rbase/.bin.mv.sh) nohist f(4242461123) cmpsize(756)\n\
             f 0644 root sys .cshrc work/irix/cmd/adm/root.cshrc eoe1.sw.unix sum(61435) size(601) f(940005084) config(suggest) cmpsize(465)\n\
             l 0777 root sys usr/lib/libc.so work/irix/lib eoe1.sw.unix symval(../lib32/libc.so)\n",
        );
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        assert_eq!(entries.len(), 4);

        let dir = &entries[0];
        assert_eq!(dir.file_type, FileType::Directory);
        assert!(dir.path.is_root());
        assert_eq!(dir.subsystem.to_string(), "eoe1.sw.unix");

        let script = &entries[1];
        assert_eq!(script.mode, 0o755);
        assert_eq!(script.path.as_str(), ".bin.mv.sh");
        assert_eq!(script.size(), Some(1426));
        assert_eq!(script.compressed_size(), Some(756));
        assert_eq!(script.encoded_size(), Some(756));

        let cshrc = &entries[2];
        assert_eq!(cshrc.config_mode(), Some(ConfigMode::Suggest));
        assert!(
            cshrc
                .attributes
                .iter()
                .any(|a| matches!(a, IdbAttribute::Unknown { name, .. } if name == "f"))
        );

        let link = &entries[3];
        assert_eq!(link.file_type, FileType::SymbolicLink);
        assert_eq!(link.symlink_target(), Some("../lib32/libc.so"));
    }

    #[test]
    fn entry_ids_follow_idb_order() {
        let (entries, _) = parse_idb(
            "f 0644 root sys a src/a eoe1.sw.unix size(1) cmpsize(0)\n\
             f 0644 root sys b src/b eoe1.sw.unix size(2) cmpsize(0)\n",
        );
        assert_eq!(entries[0].id, EntryId(0));
        assert_eq!(entries[1].id, EntryId(1));
        assert_eq!(entries[0].encoded_size(), Some(1));
    }

    #[test]
    fn malformed_lines_become_diagnostics() {
        let (entries, diagnostics) = parse_idb(
            "f 0644 root sys\n\
             f 0644 root sys ok src/ok eoe1.sw.unix size(1) cmpsize(0)\n",
        );
        assert_eq!(entries.len(), 1);
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(entries[0].origin.line_number, 2);
    }

    #[test]
    fn rejects_traversal_paths() {
        let (entries, diagnostics) =
            parse_idb("f 0644 root sys ../escape src/x eoe1.sw.unix size(1) cmpsize(0)\n");
        assert!(entries.is_empty());
        assert_eq!(diagnostics.len(), 1);
    }

    #[test]
    fn accepts_trailing_subsystem_order() {
        // Older media append the subsystem after the attribute list.
        let (entries, diagnostics) = parse_idb(
            "f 0444 root sys usr/lib/libvfr.a vfr/lib/src/libvfr.a sum(5488) size(334964) f(3663227833) cmpsize(163499) vfr.sw.vfr\n\
             d 0755 adm adm usr/adm/.profile work/irix/cmd/acct delhist eoe2.sw.acct\n",
        );
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].subsystem.to_string(), "vfr.sw.vfr");
        assert_eq!(entries[0].compressed_size(), Some(163499));
        assert_eq!(entries[1].subsystem.to_string(), "eoe2.sw.acct");
        assert!(
            entries[1]
                .attributes
                .iter()
                .any(|a| matches!(a, IdbAttribute::DeleteHistory))
        );
    }
}
