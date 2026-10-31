//! Builds the dictionary files of a pack from kaikki.org extracts. A tool, not part of any app.
//!
//! ```text
//! dawn-dict-build to-english   <lang> <words-of-lang.jsonl|->                       <out.dict>   e.g. German to English
//! dawn-dict-build from-english <lang> <words-of-lang.jsonl> <english.jsonl|->        <out.dict>   e.g. English to German
//! ```
//!
//! `words-of-lang.jsonl` is the kaikki.org extract of the language's words (`kaikki.org-dictionary-German.jsonl`);
//! `english.jsonl` the extract of the English words (translations and inflected forms). `-` reads standard input, so a
//! multi-gigabyte extract can be streamed without storing it. See `docs/dictionary.md`.

use std::io::{BufRead, BufReader};

use dawn_dict::kaikki::Collector;

fn lines(path: &str) -> Box<dyn BufRead> {
    if path == "-" {
        Box::new(BufReader::with_capacity(1 << 20, std::io::stdin()))
    } else {
        Box::new(BufReader::with_capacity(1 << 20, std::fs::File::open(path).unwrap_or_else(|e| panic!("cannot open {path}: {e}"))))
    }
}

/// Call `f` with every line of the extract (a line cut by an interrupted download is read lossily, then fails to parse and is counted).
fn for_each_line(path: &str, mut f: impl FnMut(&str)) {
    let mut reader = lines(path);
    let mut buf = Vec::new();
    loop {
        buf.clear();
        match reader.read_until(b'\n', &mut buf) {
            Ok(0) => break,
            Ok(_) => {}
            Err(e) => {
                eprintln!("read error: {e}");
                break;
            }
        }
        let text = String::from_utf8_lossy(&buf);
        let text = text.trim_end();
        if !text.is_empty() {
            f(text);
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let ok = matches!((args.first().map(String::as_str), args.len()), (Some("to-english"), 4) | (Some("from-english"), 5));
    if !ok {
        eprintln!("usage: dawn-dict-build to-english <lang> <extract|-> <out.dict>\n       dawn-dict-build from-english <lang> <words-of-lang.jsonl> <english.jsonl|-> <out.dict>");
        std::process::exit(2);
    }
    let lang = &args[1];
    let mut collector = Collector::new();
    let (source, target, out) = if args[0] == "to-english" {
        for_each_line(&args[2], |line| {
            collector.feed_glosses(line, lang);
            if collector.lines % 200_000 == 0 {
                eprintln!("{} lines", collector.lines);
            }
        });
        (lang.as_str(), "en", &args[3])
    } else {
        for_each_line(&args[2], |line| collector.feed_reverse(line, lang));
        eprintln!("{} lines read, {} unreadable", collector.lines, collector.skipped_lines);
        for_each_line(&args[3], |line| {
            collector.feed_translations(line, lang);
            if collector.lines % 200_000 == 0 {
                eprintln!("{} lines", collector.lines);
            }
        });
        ("en", lang.as_str(), &args[4])
    };
    eprintln!("{} lines in all, {} unreadable", collector.lines, collector.skipped_lines);
    let writer = collector.finish(source, target);
    let (entries, keys) = (writer.entry_count(), writer.key_count());
    let bytes = writer.finish();
    std::fs::write(out, &bytes).expect("cannot write the dictionary file");
    println!("{out}: {entries} entries, {keys} keys, {} bytes", bytes.len());
}
