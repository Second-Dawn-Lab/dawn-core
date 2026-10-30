//! Research: which model translates Urdu and Japanese back into English best?
//!
//! The sources are machine translations of the 10 sentences of the quality test (so the
//! intended English is known): Urdu from the tc-bible `-inc` model and from M2M-100, Japanese
//! from M2M-100. Candidates are compared by reading their English against the original.
//!
//! ```text
//! DAWN_RESEARCH_DIR=...\tools\convert-models\out\_research \
//!   cargo test -p dawn-translate --test reverse -- --ignored --nocapture --test-threads=1
//! ```
//!
//! Every `R|` line is `model | direction | sentence number | source | English`.

use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

use dawn_models::{InstallOptions, Model, ModelStore};
use dawn_translate::LangTokenizer;
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

const URDU_FROM_INC: [&str; 10] = [
    "گڈ مارننگ! آپ کل رات کیسے سوئے؟",
    "جمعرات کو صبح 3:30 بجے اجلاس منتقل کردیا گیا ہے ، لہذا براہ کرم اپنے کیلنڈر کو اپ ڈیٹ کریں۔",
    "میں اپنی چابیاں کہیں بھی نہیں ڈھونڈ سکتا ، اور میں پہلے ہی کام کے لئے دیر ہو چکی ہوں۔",
    "آفس سے نکلتے وقت لائٹس بند کرنا نہ بھولیں۔",
    "وہ چھ سال سے طبی تعلیم حاصل کر رہی ہیں لیکن ابھی تک اس بات کا یقین نہیں ہے کہ یہ صحیح کیریئر ہے۔",
    "اگر کل بارش ہو تو ہم پکنک کو منسوخ کر دیں گے اور اس کے بجائے گھر میں رہیں گے۔",
    "نیا سافٹ ویئر اپ ڈیٹ کئی سیکورٹی کے مسائل کو حل کرتا ہے اور تقریبا 15 فیصد کی بیٹری کی زندگی کو بہتر بناتا ہے.",
    "کیا آپ مجھے بتا سکتے ہیں کہ قریب ترین ٹرین اسٹیشن تک کیسے پہنچیں؟",
    "انہوں نے کہا کہ وہ مجھے فون کریں گے، لیکن انہوں نے کبھی نہیں کیا.",
    "ایک نئی زبان سیکھنا صبر، مشق، اور بہت تجسس لیتا ہے.",
];

const URDU_FROM_M2M: [&str; 10] = [
    "اچھا دن۔آپ نے رات کو کیسے سوتے تھے؟",
    "جمعرات کو 3:30 بجے کی طرف منتقل کیا گیا ہے، لہذا براہ مہربانی اپنے کیلنڈر کو اپ ڈیٹ کریں.",
    "میں اپنی کلیدوں کو کہیں بھی نہیں پایا، اور میں پہلے ہی کام کرنے کے لئے دیر کر رہا ہوں.",
    "براہ مہربانی دفتر سے باہر نکلنے کے لئے چراغوں کو بند کرنے کے لئے مت بھولنا.",
    "وہ چھ سال سے میڈیکل مطالعہ کر رہی ہے، لیکن وہ ابھی بھی صحیح کاروبار ہے کہ یقین نہیں ہے.",
    "اگر صبح بارش ہو تو ہم گھر میں رہیں گے.",
    "نئے سافٹ ویئر اپ ڈیٹ کئی سیکورٹی مسائل کو حل کرتا ہے اور بیٹری کی زندگی کو تقریبا 15 فیصد بہتر کرتا ہے.",
    "کیا آپ مجھے سب سے قریب ترین ٹرین سٹیشن تک پہنچنے کے لئے کس طرح بتائیں گے؟",
    "انہوں نے کہا کہ وہ مجھے واپس فون کرے گا، لیکن انہوں نے کبھی نہیں کیا.",
    "ایک نئی زبان سیکھنے کے لئے صبر، عمل، اور بہت دلچسپی کی ضرورت ہے.",
];

const JAPANESE_FROM_M2M: [&str; 10] = [
    "こんばんは!昨夜はどう寝たの?",
    "会議は木曜日午後3時30分に延期しましたので、日程を更新してください。",
    "鍵はどこにも見つからないし、仕事に遅刻している。",
    "オフィスを出るときに灯りを消すことを忘れないでください。",
    "医学を6年間勉強しているが、まだ正しい職業だとは確信していない。",
    "明日雨が降ったら、帰宅してピクニックをキャンセルします。",
    "新しいソフトウェアアップデートは、いくつかのセキュリティ問題を修正し、バッテリー寿命を約15%改善します。",
    "最寄りの駅までどうやって行くか教えてください。",
    "彼は私を呼び戻すだろうと言ったが、決してそうはしなかった。",
    "新しい言語を学ぶには、忍耐力、練習、好奇心が必要です。",
];

fn median(mut runs: Vec<Duration>) -> Duration {
    runs.sort();
    runs[runs.len() / 2]
}

fn working_set_mb() -> u64 {
    let script = format!("(Get-Process -Id {}).WorkingSet64", std::process::id());
    let out = std::process::Command::new("powershell").args(["-NoProfile", "-Command", &script]).output().unwrap();
    String::from_utf8_lossy(&out.stdout).trim().parse::<u64>().unwrap_or(0) / 1_000_000
}

/// Run `sources` through the model in `dir`, optionally with a `>>xxx<<` token, and print the English.
fn run(label: &str, direction: &str, dir: &PathBuf, prefix: Option<&str>, sources: &[&str]) {
    let before = working_set_mb();
    let config = Config { num_threads_per_replica: 4, ..Config::default() };
    let translator = Translator::with_tokenizer(dir, LangTokenizer::open(dir).unwrap(), &config).unwrap();
    let options = TranslationOptions::<String, String> { beam_size: 2, max_decoding_length: 256, ..Default::default() };
    let go = |text: &str| -> String {
        let input = match prefix {
            Some(p) => format!("{p} {text}"),
            None => text.to_string(),
        };
        translator.translate_batch(&[input], &options, None).unwrap().remove(0).0.trim().to_string()
    };
    go("Warm up."); // the first call after loading is slower
    let mut times = Vec::new();
    for (i, source) in sources.iter().enumerate() {
        let t = Instant::now();
        let out = go(source);
        times.push(t.elapsed());
        println!("R|{label}|{direction}|{:02}|{source}|{out}", i + 1);
    }
    println!("R|{label}|{direction}|timing|median {} ms per sentence|memory {} MB (before load {before} MB)", median(times).as_millis(), working_set_mb());
}

#[test]
#[ignore = "needs converted models (DAWN_RESEARCH_DIR)"]
fn reverse_models() {
    let research = PathBuf::from(std::env::var("DAWN_RESEARCH_DIR").expect("set DAWN_RESEARCH_DIR"));

    // The previous Opus-MT reverse models, installed from their converted archives (they are no
    // longer in the catalog).
    let root = std::env::temp_dir().join(format!("dawn-reverse-store-{}", std::process::id()));
    let store = ModelStore::new(root);
    let cancel = AtomicBool::new(false);
    let archives = research.parent().expect("research dir has a parent").to_path_buf();
    for (id, source, target) in [("opus-mt-ur-en-ct2-int8", "ur", "en"), ("opus-mt-ja-en-ct2-int8", "ja", "en")] {
        let zip = archives.join(format!("{id}.zip"));
        let model = Model {
            id: id.to_string(),
            sources: vec![source.to_string()],
            targets: vec![target.to_string()],
            urls: vec![format!("file:///{}", zip.to_string_lossy().replace('\\', "/"))],
            size_bytes: std::fs::metadata(&zip).unwrap().len(),
            sha256: dawn_models::sha256_file(&zip).unwrap(),
            license: "Apache-2.0".into(),
            attribution: "Helsinki-NLP / OPUS-MT".into(),
            prefix: None,
            target_tokens: Default::default(),
        };
        store.install_model(&model, &InstallOptions { proxy: None, cancel: &cancel }, &mut |_| {}).unwrap();
    }

    println!("R|english|reference|{}", ENGLISH.join(" || "));
    for (set, sources) in [("urdu from -inc", &URDU_FROM_INC), ("urdu from M2M-100", &URDU_FROM_M2M)] {
        run("current opus-mt-ur-en", set, &store.model_dir("opus-mt-ur-en-ct2-int8"), None, sources);
        run("tc-bible-big-inc-en", set, &research.join("inc-en-ct2-int8"), None, sources);
    }
    run("current opus-mt-ja-en", "japanese from M2M-100", &store.model_dir("opus-mt-ja-en-ct2-int8"), None, &JAPANESE_FROM_M2M);
    run("tc-bible-big-mul (>>eng<<)", "japanese from M2M-100", &research.join("mul-rev-ct2-int8"), Some(">>eng<<"), &JAPANESE_FROM_M2M);
}
