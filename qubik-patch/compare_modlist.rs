//! Compare the manifests and mod lists in two modpack directories.
//!
//! Usage:
//!   rustc qubik-patch/compare_modlist.rs -O -o /tmp/compare_modlist
//!   /tmp/compare_modlist <upstream-modpack-dir> <patched-modpack-dir>
//!
//! Comparison semantics:
//!   - Reads `manifest.json` and `modlist.html` from each modpack directory.
//!   - Compares each file type independently using CurseForge project IDs.
//!   - Succeeds only when both pairs of unique-ID sets match and neither input contains duplicates.

use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::fs;
use std::path::Path;
use std::process;

const CURSEFORGE_PROJECT_URL_MARKER: &str = "curseforge.com/projects/";
const MANIFEST_FILE_NAME: &str = "manifest.json";
const MODLIST_FILE_NAME: &str = "modlist.html";

fn main() {
    match run() {
        Ok(code) => process::exit(code),
        Err(message) => {
            eprintln!("error: {message}");
            process::exit(1);
        }
    }
}

fn run() -> Result<i32, String> {
    let args: Vec<String> = env::args().collect();
    if args.get(1).is_some_and(|arg| arg == "-h" || arg == "--help") {
        print_help(&args[0]);
        return Ok(0);
    }
    if args.len() != 3 {
        return Err(format!(
            "expected exactly 2 modpack directory arguments\n\n{}",
            usage_text(&args[0])
        ));
    }

    let upstream_dir = Path::new(&args[1]);
    let patched_dir = Path::new(&args[2]);
    let upstream_manifest = load_report(
        upstream_dir,
        MANIFEST_FILE_NAME,
        "manifest",
        extract_manifest_ids,
    )?;
    let patched_manifest = load_report(
        patched_dir,
        MANIFEST_FILE_NAME,
        "manifest",
        extract_manifest_ids,
    )?;
    let upstream_modlist = load_report(
        upstream_dir,
        MODLIST_FILE_NAME,
        "mod list",
        extract_modlist_ids,
    )?;
    let patched_modlist = load_report(
        patched_dir,
        MODLIST_FILE_NAME,
        "mod list",
        extract_modlist_ids,
    )?;

    let only_in_upstream_manifest =
        sorted_difference(&upstream_manifest.unique_ids, &patched_manifest.unique_ids);
    let only_in_patched_manifest =
        sorted_difference(&patched_manifest.unique_ids, &upstream_manifest.unique_ids);
    let only_in_upstream_modlist =
        sorted_difference(&upstream_modlist.unique_ids, &patched_modlist.unique_ids);
    let only_in_patched_modlist =
        sorted_difference(&patched_modlist.unique_ids, &upstream_modlist.unique_ids);
    let success = upstream_manifest.duplicates.is_empty()
        && patched_manifest.duplicates.is_empty()
        && upstream_modlist.duplicates.is_empty()
        && patched_modlist.duplicates.is_empty()
        && only_in_upstream_manifest.is_empty()
        && only_in_patched_manifest.is_empty()
        && only_in_upstream_modlist.is_empty()
        && only_in_patched_modlist.is_empty();

    println!("Upstream manifest project IDs:");
    print_source_summary("file entries", &upstream_manifest);
    println!();
    println!("Patched manifest project IDs:");
    print_source_summary("file entries", &patched_manifest);
    println!();
    println!(
        "Only in upstream manifest: {}",
        format_id_list(&only_in_upstream_manifest)
    );
    println!(
        "Only in patched manifest: {}",
        format_id_list(&only_in_patched_manifest)
    );
    println!();
    println!("Upstream mod list project IDs:");
    print_source_summary("CurseForge links", &upstream_modlist);
    println!();
    println!("Patched mod list project IDs:");
    print_source_summary("CurseForge links", &patched_modlist);
    println!();
    println!(
        "Only in upstream mod list: {}",
        format_id_list(&only_in_upstream_modlist)
    );
    println!(
        "Only in patched mod list: {}",
        format_id_list(&only_in_patched_modlist)
    );
    println!();
    if success {
        println!("Result: both modpack directories match and contain no duplicates.");
        Ok(0)
    } else {
        println!("Result: differences or duplicates detected.");
        Ok(1)
    }
}

fn print_help(program: &str) {
    println!("{}", usage_text(program));
    println!();
    println!("Comparison semantics:");
    println!("  - Reads project IDs only from manifest files[].projectID");
    println!("  - Ignores the pack-level manifest projectID and manifest fileID values");
    println!("  - Reads IDs only from CurseForge /projects/<id> links in modlist.html");
    println!("  - Compares manifest.json and modlist.html independently across both directories");
    println!("  - Exits non-zero for duplicates, differences, unreadable files, or invalid input");
}

fn usage_text(program: &str) -> String {
    format!(
        "Usage:\n  {program} <upstream-modpack-dir> <patched-modpack-dir>\n\nExample:\n  rustc qubik-patch/compare_modlist.rs -O -o /tmp/compare_modlist\n  /tmp/compare_modlist /path/to/upstream /path/to/patched"
    )
}

fn load_report(
    modpack_dir: &Path,
    file_name: &str,
    file_description: &str,
    extract_ids: fn(&str) -> Result<IdReport, String>,
) -> Result<IdReport, String> {
    if !modpack_dir.is_dir() {
        return Err(format!(
            "modpack directory '{}' does not exist or is not a directory",
            modpack_dir.display()
        ));
    }

    let path = modpack_dir.join(file_name);
    let source = fs::read_to_string(&path).map_err(|error| {
        format!(
            "failed to read {file_description} '{}': {error}",
            path.display()
        )
    })?;
    extract_ids(&source)
        .map_err(|error| format!("invalid {file_description} '{}': {error}", path.display()))
}

fn print_source_summary(label: &str, report: &IdReport) {
    println!("  total {label}: {}", report.total_count);
    println!("  unique IDs: {}", report.unique_ids.len());
    println!("  duplicate IDs: {}", format_duplicates(&report.duplicates));
}

fn format_duplicates(duplicates: &[(u64, usize)]) -> String {
    if duplicates.is_empty() {
        return "none".to_string();
    }
    duplicates
        .iter()
        .map(|(id, count)| format!("{id} ({count}x)"))
        .collect::<Vec<_>>()
        .join(", ")
}

fn format_id_list(ids: &[u64]) -> String {
    if ids.is_empty() {
        return "none".to_string();
    }
    ids.iter()
        .map(u64::to_string)
        .collect::<Vec<_>>()
        .join(", ")
}

fn sorted_difference(left: &BTreeSet<u64>, right: &BTreeSet<u64>) -> Vec<u64> {
    left.difference(right).copied().collect()
}

fn extract_modlist_ids(source: &str) -> Result<IdReport, String> {
    let lowercase = source.to_ascii_lowercase();
    let bytes = lowercase.as_bytes();
    let mut counts = BTreeMap::new();
    let mut search_from = 0usize;

    while let Some(relative_offset) = lowercase[search_from..].find(CURSEFORGE_PROJECT_URL_MARKER) {
        let digits_start = search_from + relative_offset + CURSEFORGE_PROJECT_URL_MARKER.len();
        let mut digits_end = digits_start;
        while digits_end < bytes.len() && bytes[digits_end].is_ascii_digit() {
            digits_end += 1;
        }
        if digits_end == digits_start {
            return Err(format!(
                "found CurseForge projects URL without a numeric project ID near byte {digits_start}"
            ));
        }
        let id = lowercase[digits_start..digits_end]
            .parse::<u64>()
            .map_err(|error| format!("invalid project ID near byte {digits_start}: {error}"))?;
        *counts.entry(id).or_insert(0) += 1;
        search_from = digits_end;
    }

    Ok(report_from_counts(counts))
}

fn extract_manifest_ids(source: &str) -> Result<IdReport, String> {
    let mut parser = JsonParser::new(source);
    let counts = parser.parse_manifest_files()?;
    parser.skip_whitespace();
    if !parser.is_eof() {
        return Err(format!(
            "unexpected trailing content at byte {}",
            parser.position()
        ));
    }
    Ok(report_from_counts(counts))
}

fn report_from_counts(counts: BTreeMap<u64, usize>) -> IdReport {
    let total_count = counts.values().sum();
    let unique_ids = counts.keys().copied().collect();
    let duplicates = counts
        .into_iter()
        .filter(|(_, count)| *count > 1)
        .collect::<Vec<_>>();
    IdReport {
        total_count,
        unique_ids,
        duplicates,
    }
}

#[derive(Debug)]
struct IdReport {
    total_count: usize,
    unique_ids: BTreeSet<u64>,
    duplicates: Vec<(u64, usize)>,
}

struct JsonParser<'a> {
    input: &'a [u8],
    position: usize,
}

impl<'a> JsonParser<'a> {
    fn new(input: &'a str) -> Self {
        Self {
            input: input.as_bytes(),
            position: 0,
        }
    }

    fn parse_manifest_files(&mut self) -> Result<BTreeMap<u64, usize>, String> {
        self.skip_whitespace();
        self.expect_byte(b'{')?;
        let mut files = None;

        loop {
            self.skip_whitespace();
            if self.try_consume(b'}') {
                break;
            }

            let key = self.parse_string()?;
            self.skip_whitespace();
            self.expect_byte(b':')?;
            self.skip_whitespace();

            if key == "files" {
                if files.is_some() {
                    return Err("duplicate top-level 'files' key".to_string());
                }
                files = Some(self.parse_files_array()?);
            } else {
                self.skip_value()?;
            }

            self.skip_whitespace();
            if self.try_consume(b',') {
                continue;
            }
            self.expect_byte(b'}')?;
            break;
        }

        files.ok_or_else(|| "missing top-level 'files' array".to_string())
    }

    fn parse_files_array(&mut self) -> Result<BTreeMap<u64, usize>, String> {
        self.expect_byte(b'[')?;
        let mut counts = BTreeMap::new();

        loop {
            self.skip_whitespace();
            if self.try_consume(b']') {
                break;
            }

            let project_id = self.parse_file_entry()?;
            *counts.entry(project_id).or_insert(0) += 1;

            self.skip_whitespace();
            if self.try_consume(b',') {
                continue;
            }
            self.expect_byte(b']')?;
            break;
        }

        Ok(counts)
    }

    fn parse_file_entry(&mut self) -> Result<u64, String> {
        self.expect_byte(b'{')?;
        let mut project_id = None;

        loop {
            self.skip_whitespace();
            if self.try_consume(b'}') {
                break;
            }

            let key = self.parse_string()?;
            self.skip_whitespace();
            self.expect_byte(b':')?;
            self.skip_whitespace();

            if key == "projectID" {
                if project_id.is_some() {
                    return Err(format!(
                        "duplicate 'projectID' key in files entry at byte {}",
                        self.position
                    ));
                }
                project_id = Some(self.parse_u64_number()?);
            } else {
                self.skip_value()?;
            }

            self.skip_whitespace();
            if self.try_consume(b',') {
                continue;
            }
            self.expect_byte(b'}')?;
            break;
        }

        project_id.ok_or_else(|| "files entry is missing numeric 'projectID'".to_string())
    }

    fn parse_u64_number(&mut self) -> Result<u64, String> {
        let start = self.position;
        let token = self.parse_number_token()?;
        if token.starts_with('-') {
            return Err(format!(
                "expected a non-negative integer project ID at byte {start}"
            ));
        }
        if token.contains('.') || token.contains('e') || token.contains('E') {
            return Err(format!("expected an integer project ID at byte {start}"));
        }
        token
            .parse::<u64>()
            .map_err(|error| {
                format!("invalid integer project ID '{token}' at byte {start}: {error}")
            })
    }

    fn parse_number_token(&mut self) -> Result<String, String> {
        let start = self.position;
        if self.try_consume(b'-')
            && !matches!(self.peek_byte(), Some(b'0'..=b'9'))
        {
            return Err(format!("expected digits after '-' at byte {start}"));
        }

        self.consume_integer_digits(start)?;

        if self.try_consume(b'.') {
            let fractional_start = self.position;
            self.consume_digits(fractional_start)?;
        }

        if self.peek_byte_is(b'e') || self.peek_byte_is(b'E') {
            self.position += 1;
            if self.peek_byte_is(b'+') || self.peek_byte_is(b'-') {
                self.position += 1;
            }
            let exponent_start = self.position;
            self.consume_digits(exponent_start)?;
        }

        String::from_utf8(self.input[start..self.position].to_vec())
            .map_err(|error| format!("invalid UTF-8 in number token: {error}"))
    }

    fn consume_integer_digits(&mut self, start: usize) -> Result<(), String> {
        match self.peek_byte() {
            Some(b'0') => {
                self.position += 1;
                if self.peek_byte().is_some_and(|byte| byte.is_ascii_digit()) {
                    return Err(format!("leading zero in number at byte {start}"));
                }
                Ok(())
            }
            Some(b'1'..=b'9') => self.consume_digits(start),
            _ => Err(format!("expected a JSON number at byte {start}")),
        }
    }

    fn consume_digits(&mut self, start: usize) -> Result<(), String> {
        let mut consumed = false;
        while let Some(byte) = self.peek_byte() {
            if byte.is_ascii_digit() {
                consumed = true;
                self.position += 1;
            } else {
                break;
            }
        }
        if consumed {
            Ok(())
        } else {
            Err(format!("expected digits at byte {start}"))
        }
    }

    fn parse_string(&mut self) -> Result<String, String> {
        self.expect_byte(b'"')?;
        let mut output = None::<Vec<u8>>;
        let mut segment_start = self.position;

        loop {
            let byte = self
                .peek_byte()
                .ok_or_else(|| "unterminated string".to_string())?;
            match byte {
                b'"' => {
                    let segment = &self.input[segment_start..self.position];
                    self.position += 1;
                    if let Some(mut bytes) = output {
                        bytes.extend_from_slice(segment);
                        return String::from_utf8(bytes)
                            .map_err(|error| format!("invalid UTF-8 in string: {error}"));
                    }
                    return std::str::from_utf8(segment)
                        .map(|text| text.to_owned())
                        .map_err(|error| format!("invalid UTF-8 in string: {error}"));
                }
                b'\\' => {
                    let output = output.get_or_insert_with(Vec::new);
                    output.extend_from_slice(&self.input[segment_start..self.position]);
                    self.position += 1;
                    let escaped = self
                        .peek_byte()
                        .ok_or_else(|| "unterminated escape sequence".to_string())?;
                    self.position += 1;
                    match escaped {
                        b'"' => output.push(b'"'),
                        b'\\' => output.push(b'\\'),
                        b'/' => output.push(b'/'),
                        b'b' => output.push(0x08),
                        b'f' => output.push(0x0c),
                        b'n' => output.push(b'\n'),
                        b'r' => output.push(b'\r'),
                        b't' => output.push(b'\t'),
                        b'u' => {
                            let scalar = self.parse_unicode_escape()?;
                            let mut buffer = [0u8; 4];
                            output.extend_from_slice(scalar.encode_utf8(&mut buffer).as_bytes());
                        }
                        _ => return Err(format!("invalid escape sequence '\\{}'", escaped as char)),
                    }
                    segment_start = self.position;
                }
                0x00..=0x1f => {
                    return Err(format!(
                        "unescaped control character in string at byte {}",
                        self.position
                    ))
                }
                _ => self.position += 1,
            }
        }
    }

    fn parse_unicode_escape(&mut self) -> Result<char, String> {
        let start = self.position;
        if self.position + 4 > self.input.len() {
            return Err("incomplete unicode escape".to_string());
        }
        let digits = std::str::from_utf8(&self.input[self.position..self.position + 4])
            .map_err(|error| format!("invalid unicode escape bytes: {error}"))?;
        self.position += 4;
        let value = u32::from_str_radix(digits, 16)
            .map_err(|error| format!("invalid unicode escape '\\u{digits}': {error}"))?;
        char::from_u32(value).ok_or_else(|| format!("invalid unicode scalar at byte {start}"))
    }

    fn skip_value(&mut self) -> Result<(), String> {
        self.skip_whitespace();
        match self.peek_byte() {
            Some(b'"') => {
                self.skip_string()?;
                Ok(())
            }
            Some(b'{') => self.skip_object(),
            Some(b'[') => self.skip_array(),
            Some(b'-' | b'0'..=b'9') => {
                self.parse_number_token()?;
                Ok(())
            }
            Some(b't') => self.expect_keyword(b"true"),
            Some(b'f') => self.expect_keyword(b"false"),
            Some(b'n') => self.expect_keyword(b"null"),
            Some(byte) => Err(format!(
                "unexpected byte '{}' while parsing JSON value at byte {}",
                byte as char, self.position
            )),
            None => Err("unexpected end of input while parsing JSON value".to_string()),
        }
    }

    fn skip_array(&mut self) -> Result<(), String> {
        self.expect_byte(b'[')?;
        loop {
            self.skip_whitespace();
            if self.try_consume(b']') {
                return Ok(());
            }
            self.skip_value()?;
            self.skip_whitespace();
            if self.try_consume(b',') {
                continue;
            }
            self.expect_byte(b']')?;
            return Ok(());
        }
    }

    fn skip_object(&mut self) -> Result<(), String> {
        self.expect_byte(b'{')?;
        loop {
            self.skip_whitespace();
            if self.try_consume(b'}') {
                return Ok(());
            }
            self.skip_string()?;
            self.skip_whitespace();
            self.expect_byte(b':')?;
            self.skip_value()?;
            self.skip_whitespace();
            if self.try_consume(b',') {
                continue;
            }
            self.expect_byte(b'}')?;
            return Ok(());
        }
    }

    fn skip_string(&mut self) -> Result<(), String> {
        self.expect_byte(b'"')?;
        loop {
            let byte_position = self.position;
            let byte = self
                .peek_byte()
                .ok_or_else(|| "unterminated string".to_string())?;
            self.position += 1;
            match byte {
                b'"' => return Ok(()),
                b'\\' => {
                    let escaped = self
                        .peek_byte()
                        .ok_or_else(|| "unterminated escape sequence".to_string())?;
                    self.position += 1;
                    if escaped == b'u' {
                        if self.position + 4 > self.input.len() {
                            return Err("incomplete unicode escape".to_string());
                        }
                        self.position += 4;
                    }
                }
                0x00..=0x1f => {
                    return Err(format!(
                        "unescaped control character in string at byte {}",
                        byte_position
                    ))
                }
                _ => {}
            }
        }
    }

    fn expect_keyword(&mut self, keyword: &[u8]) -> Result<(), String> {
        if self
            .input
            .get(self.position..)
            .is_some_and(|remaining| remaining.starts_with(keyword))
        {
            self.position += keyword.len();
            Ok(())
        } else {
            Err(format!(
                "expected '{}' at byte {}",
                String::from_utf8_lossy(keyword),
                self.position
            ))
        }
    }

    fn expect_byte(&mut self, expected: u8) -> Result<(), String> {
        match self.peek_byte() {
            Some(byte) if byte == expected => {
                self.position += 1;
                Ok(())
            }
            Some(byte) => Err(format!(
                "expected '{}' at byte {}, found '{}'",
                expected as char, self.position, byte as char
            )),
            None => Err(format!(
                "expected '{}' at byte {}, found end of input",
                expected as char, self.position
            )),
        }
    }

    fn try_consume(&mut self, expected: u8) -> bool {
        if self.peek_byte() == Some(expected) {
            self.position += 1;
            true
        } else {
            false
        }
    }

    fn peek_byte(&self) -> Option<u8> {
        self.input.get(self.position).copied()
    }

    fn peek_byte_is(&self, expected: u8) -> bool {
        self.peek_byte() == Some(expected)
    }

    fn skip_whitespace(&mut self) {
        while let Some(byte) = self.peek_byte() {
            if matches!(byte, b' ' | b'\n' | b'\r' | b'\t') {
                self.position += 1;
            } else {
                break;
            }
        }
    }

    fn is_eof(&self) -> bool {
        self.position >= self.input.len()
    }

    fn position(&self) -> usize {
        self.position
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    struct TestDirectoryGuard(std::path::PathBuf);

    impl Drop for TestDirectoryGuard {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn loads_manifest_and_modlist_from_a_modpack_directory() {
        let directory = TestDirectoryGuard(std::env::temp_dir().join(format!(
            "compare_modlist_test_{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("current time should be after the Unix epoch")
                .as_nanos()
        )));
        fs::create_dir(&directory.0).expect("test directory should be created");
        fs::write(
            directory.0.join(MANIFEST_FILE_NAME),
            r#"{"files":[{"projectID":100},{"projectID":200}]}"#,
        )
        .expect("manifest should be written");
        fs::write(
            directory.0.join(MODLIST_FILE_NAME),
            r#"<a href="https://www.curseforge.com/projects/100">one</a>"#,
        )
        .expect("mod list should be written");

        let manifest = load_report(
            &directory.0,
            MANIFEST_FILE_NAME,
            "manifest",
            extract_manifest_ids,
        )
        .expect("manifest should load");
        let modlist = load_report(
            &directory.0,
            MODLIST_FILE_NAME,
            "mod list",
            extract_modlist_ids,
        )
        .expect("mod list should load");

        assert_eq!(manifest.unique_ids, BTreeSet::from([100, 200]));
        assert_eq!(modlist.unique_ids, BTreeSet::from([100]));
        assert_eq!(
            sorted_difference(&manifest.unique_ids, &modlist.unique_ids),
            vec![200]
        );
    }
}
