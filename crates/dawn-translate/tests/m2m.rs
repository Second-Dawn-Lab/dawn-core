//! Evaluation of larger multilingual models for English to Urdu / Japanese (research, not used
//! by the app). Needs models converted with `ct2-transformers-converter --quantization int8`:
//!
//! ```text
//! DAWN_M2M_DIR=...\m2m100-1.2B-ct2-int8 DAWN_BIBLE_DIR=...\opus-mt-tc-bible-big-mul-ct2-int8 \
//!   cargo test -p dawn-translate --test m2m -- --ignored --nocapture --test-threads=1
//! ```
//!
//! Every `M|` line is `model | pair | sentence number | source | translation`.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use ct2rs::tokenizers::sentencepiece::Tokenizer as Spm;
use ct2rs::{Config, TranslationOptions, Translator};

const ENGLISH: [&str; 10] = [
    "Good morning! How did you sleep last night?",
    "The meeting has been moved to Thursday at 3:30 p.m., so please update your calendar.",
    "I can't find my keys anywhere, and I'm already late for work.",
    "Please don't forget to turn off the lights when you leave the office.",
    "She has been studying medicine for six years, but she still isn't sure it's the right career.",
    "If it rains tomorrow, we will cancel the picnic and stay at home instead.",
    "The new software update fixes several security problems and improves battery life by about 15 percent.",
    "Could you tell me how to get to the nearest train station?",
    "He said he would call me back, but he never did.",
    "Learning a new language takes patience, practice, and a lot of curiosity.",
];

/// M2M-100 wants `__en__ <pieces> </s>` on the source side; the target language token goes in
/// as a forced prefix (ct2rs removes it from the output again).
struct M2mTokenizer {
    spm: Spm,
    source_lang: String,
}

impl ct2rs::Tokenizer for M2mTokenizer {
    fn encode(&self, input: &str) -> anyhow::Result<Vec<String>> {
        let mut tokens = vec![self.source_lang.clone()];
        tokens.extend(ct2rs::Tokenizer::encode(&self.spm, input)?);
        tokens.push("</s>".to_string());
        Ok(tokens)
    }

    fn decode(&self, tokens: Vec<String>) -> anyhow::Result<String> {
        ct2rs::Tokenizer::decode(&self.spm, tokens)
    }
}

/// `>>urd<< sentence` becomes the tokens `[">>urd<<", pieces of "sentence"...]`. The language
/// token is a single vocabulary entry in the tc-bible models; the SentencePiece model does not
/// know it, so encoding the whole string would split it into garbage pieces.
struct LangTokenTokenizer {
    spm: Spm,
}

impl ct2rs::Tokenizer for LangTokenTokenizer {
    fn encode(&self, input: &str) -> anyhow::Result<Vec<String>> {
        let mut tokens = Vec::new();
        let mut text = input;
        if let Some(rest) = input.strip_prefix(">>") {
            if let Some(end) = rest.find("<<") {
                tokens.push(format!(">>{}<<", &rest[..end]));
                text = rest[end + 2..].trim_start();
            }
        }
        tokens.extend(ct2rs::Tokenizer::encode(&self.spm, text)?);
        Ok(tokens)
    }

    fn decode(&self, tokens: Vec<String>) -> anyhow::Result<String> {
        ct2rs::Tokenizer::decode(&self.spm, tokens)
    }
}

fn working_set_mb() -> u64 {
    let script = format!("(Get-Process -Id {}).WorkingSet64", std::process::id());
    let out = std::process::Command::new("powershell").args(["-NoProfile", "-Command", &script]).output().unwrap();
    String::from_utf8_lossy(&out.stdout).trim().parse::<u64>().unwrap_or(0) / 1_000_000
}

fn dir_size_mb(dir: &PathBuf) -> f64 {
    std::fs::read_dir(dir).unwrap().filter_map(|e| e.ok()).filter_map(|e| e.metadata().ok()).filter(|m| m.is_file()).map(|m| m.len()).sum::<u64>() as f64 / 1e6
}

fn median(mut runs: Vec<Duration>) -> Duration {
    runs.sort();
    runs[runs.len() / 2]
}

fn options() -> TranslationOptions<String, String> {
    TranslationOptions { beam_size: 2, max_decoding_length: 256, ..Default::default() }
}

fn model_dir(var: &str) -> PathBuf {
    PathBuf::from(std::env::var(var).unwrap_or_else(|_| panic!("set {var} to the converted model folder")))
}

/// Run `sentences` one by one (like the app's quality test) and report time and memory.
fn run(label: &str, translate: &dyn Fn(&str, &str) -> String, pairs: &[(&str, &str)], before_mb: u64, size_mb: f64) {
    println!("M|{label}|disk {size_mb:.0} MB|memory after load {} MB (before {before_mb} MB)", working_set_mb());
    for (pair, target) in pairs {
        let mut times = Vec::new();
        for (i, sentence) in ENGLISH.iter().enumerate() {
            let t = Instant::now();
            let out = translate(sentence, target);
            times.push(t.elapsed());
            println!("M|{label}|{pair}|{:02}|{sentence}|{out}", i + 1);
        }
        let total: Duration = times.iter().sum();
        println!("M|{label}|{pair}|timing|median {} ms per sentence, {} ms for all 10", median(times).as_millis(), total.as_millis());
    }
}

#[test]
#[ignore = "needs converted M2M-100 model (DAWN_M2M_DIR)"]
fn m2m100_1_2b() {
    let dir = model_dir("DAWN_M2M_DIR");
    let before = working_set_mb();
    let t = Instant::now();
    let spm = Spm::from_file(dir.join("sentencepiece.bpe.model"), dir.join("sentencepiece.bpe.model")).unwrap();
    let tokenizer = M2mTokenizer { spm, source_lang: "__en__".into() };
    let config = Config { num_threads_per_replica: 4, ..Config::default() };
    let translator = Translator::with_tokenizer(&dir, tokenizer, &config).unwrap();
    println!("M|m2m100-1.2B|load time {} ms", t.elapsed().as_millis());
    let translate = |text: &str, target: &str| -> String {
        let prefixes = vec![vec![format!("__{target}__")]];
        translator.translate_batch_with_target_prefix(&[text], &prefixes, &options(), None).unwrap().remove(0).0.trim().to_string()
    };
    translate("Warm up.", "de"); // first call after loading is slower
    run("m2m100-1.2B", &translate, &[("en-ur", "ur"), ("en-ja", "ja"), ("en-de", "de")], before, dir_size_mb(&dir));
}

/// The newest Helsinki-NLP multi-target models (2024, Apache-2.0): the target language is chosen
/// by a `>>xxx<<` token in front of the sentence.
fn bible(label: &str, var: &str, pairs: &[(&str, &str)]) {
    let dir = model_dir(var);
    let before = working_set_mb();
    let t = Instant::now();
    let config = Config { num_threads_per_replica: 4, ..Config::default() };
    let spm = Spm::from_file(dir.join("source.spm"), dir.join("target.spm")).unwrap();
    let translator = Translator::with_tokenizer(&dir, LangTokenTokenizer { spm }, &config).unwrap();
    println!("M|{label}|load time {} ms", t.elapsed().as_millis());
    let translate = |text: &str, target: &str| -> String {
        let code = match target {
            "ur" => "urd",
            "ja" => "jpn",
            other => other,
        };
        translator.translate_batch(&[format!(">>{code}<< {text}")], &options(), None).unwrap().remove(0).0.trim().to_string()
    };
    // One sentence first: if this is garbage, stop before the full set.
    let probe = translate("Good morning! How did you sleep last night?", pairs[0].1);
    println!("M|{label}|probe|{probe}");
    translate("Warm up.", "de");
    if std::env::var("DAWN_PROBE_ONLY").is_ok() {
        return;
    }
    run(label, &translate, pairs, before, dir_size_mb(&dir));
}

#[test]
#[ignore = "needs converted opus-mt-tc-bible-big model (DAWN_BIBLE_DIR)"]
fn opus_mt_tc_bible_big_mul() {
    bible("opus-mt-tc-bible-big-mul", "DAWN_BIBLE_DIR", &[("en-ur", "ur"), ("en-ja", "ja")]);
}

#[test]
#[ignore = "needs converted opus-mt-tc-bible-big-inc model (DAWN_BIBLE_INC_DIR)"]
fn opus_mt_tc_bible_big_inc() {
    bible("opus-mt-tc-bible-big-inc", "DAWN_BIBLE_INC_DIR", &[("en-ur", "ur")]);
}
