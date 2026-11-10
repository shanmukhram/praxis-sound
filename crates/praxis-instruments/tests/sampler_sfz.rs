//! SFZ parser tests: headers and inheritance, the contract opcodes, `#define`, `#include`,
//! `default_path`, comments, note names, warnings and errors with line numbers.

use praxis_instruments::sampler::region::{FilterType, LoopMode, SampleRef, Trigger};
use praxis_instruments::sampler::sfz::{self, SfzError};
use std::collections::BTreeMap;

fn fixture_reader() -> impl FnMut(&str) -> Result<String, String> {
    let mut files = BTreeMap::new();
    files.insert(
        "p/sfz/synth_piano.sfz".to_string(),
        include_str!("fixtures/sampler/synth_piano.sfz").to_string(),
    );
    files.insert(
        "p/sfz/common.sfz".to_string(),
        include_str!("fixtures/sampler/common.sfz").to_string(),
    );
    move |path: &str| {
        files
            .get(path)
            .cloned()
            .ok_or_else(|| format!("no such file `{path}`"))
    }
}

fn file(s: &str) -> SampleRef {
    SampleRef::File(s.to_string())
}

#[test]
fn parses_the_fixture_with_include_define_and_default_path() {
    let inst = sfz::parse_file("p/sfz/synth_piano.sfz", &mut fixture_reader()).unwrap();
    assert!(inst.warnings.is_empty(), "{:?}", inst.warnings);
    let r = &inst.regions;
    assert_eq!(r.len(), 6);

    // Velocity layers: group key range (note names), $defines, inherited release.
    assert_eq!(r[0].sample, file("p/samples/soft_a3.wav"));
    assert_eq!((r[0].lokey, r[0].hikey, r[0].pitch_keycenter), (48, 62, 57));
    assert_eq!((r[0].lovel, r[0].hivel), (0, 64));
    assert_eq!((r[1].lovel, r[1].hivel), (65, 127));
    assert_eq!(r[1].volume, -1.0);
    assert_eq!(r[0].ampeg.release, 0.3);
    assert_eq!(r[0].amp_veltrack, 100.0);

    // Loop opcodes from a continued group line.
    assert_eq!((r[2].lokey, r[2].hikey, r[2].pitch_keycenter), (63, 70, 69));
    assert_eq!(r[2].loop_mode, Some(LoopMode::Continuous));
    assert_eq!(
        (r[2].loop_start, r[2].loop_end),
        (Some(44_000), Some(87_999))
    );

    // Round robin; `key=` sets lokey, hikey and keycenter.
    assert_eq!((r[3].lokey, r[3].hikey, r[3].pitch_keycenter), (72, 72, 72));
    assert_eq!((r[3].seq_length, r[3].seq_position), (2, 1));
    assert_eq!((r[4].seq_length, r[4].seq_position), (2, 2));
    assert_eq!(r[4].sample, file("p/samples/rr2_c5.wav"));

    // Release trigger, group override of a global opcode, define from the included file.
    assert_eq!(r[5].trigger, Trigger::Release);
    assert_eq!(r[5].rt_decay, 6.0);
    assert_eq!(r[5].ampeg.release, 0.2);
    assert_eq!(r[5].volume, -3.0);
    assert!(r[5].origin.starts_with("p/sfz/synth_piano.sfz:"));
}

#[test]
fn all_contract_opcodes() {
    let text = "<global> volume=-6 pan=-20 amp_veltrack=50 tune=-12 transpose=1
<group> ampeg_attack=0.01 ampeg_hold=0.02 ampeg_decay=0.3 ampeg_sustain=40 ampeg_release=1.5
 ampeg_delay=0.005 offset=100 end=9000
<region> sample=s.wav lokey=f#3 hikey=g3 pitch_keycenter=54 lovel=10 hivel=90
 loop_mode=loop_sustain loop_start=200 loop_end=800 trigger=release rt_decay=3
 seq_length=3 seq_position=2 lorand=0.25 hirand=0.5
 xfin_lovel=10 xfin_hivel=30 xfout_lovel=70 xfout_hivel=90
 fil_type=lpf_1p cutoff=2000 pitch_keytrack=50
<region> sample=t.flac key=c-1 loop_mode=one_shot trigger=first
<region> sample=u.wav key=g9 loopmode=no_loop trigger=legato";
    let inst = sfz::parse_str(text, "pk/a.sfz").unwrap();
    let r = &inst.regions[0];
    assert_eq!(r.sample, file("pk/s.wav"));
    assert_eq!(
        (r.volume, r.pan, r.amp_veltrack, r.tune, r.transpose),
        (-6.0, -20.0, 50.0, -12.0, 1)
    );
    assert_eq!(r.ampeg.delay, 0.005);
    assert_eq!(r.ampeg.attack, 0.01);
    assert_eq!(r.ampeg.hold, 0.02);
    assert_eq!(r.ampeg.decay, 0.3);
    assert!((r.ampeg.sustain - 0.4).abs() < 1e-6);
    assert_eq!(r.ampeg.release, 1.5);
    assert_eq!((r.offset, r.end), (100, Some(9000)));
    assert_eq!((r.lokey, r.hikey, r.pitch_keycenter), (54, 55, 54));
    assert_eq!((r.lovel, r.hivel), (10, 90));
    assert_eq!(r.loop_mode, Some(LoopMode::Sustain));
    assert_eq!((r.loop_start, r.loop_end), (Some(200), Some(800)));
    assert_eq!((r.trigger, r.rt_decay), (Trigger::Release, 3.0));
    assert_eq!((r.seq_length, r.seq_position), (3, 2));
    assert_eq!((r.lorand, r.hirand), (0.25, 0.5));
    assert_eq!(
        (r.xfin_lovel, r.xfin_hivel, r.xfout_lovel, r.xfout_hivel),
        (10, 30, 70, 90)
    );
    assert!(r.has_explicit_xfade());
    let f = r.filter.unwrap();
    assert_eq!((f.kind, f.cutoff_hz), (FilterType::Lpf1p, 2000.0));
    assert_eq!(r.pitch_keytrack, 50.0);
    let r1 = &inst.regions[1];
    assert_eq!(
        (r1.lokey, r1.trigger, r1.loop_mode),
        (0, Trigger::First, Some(LoopMode::OneShot))
    );
    let r2 = &inst.regions[2];
    assert_eq!(
        (r2.hikey, r2.trigger, r2.loop_mode),
        (127, Trigger::Legato, Some(LoopMode::NoLoop))
    );
}

#[test]
fn defaults_follow_sfz() {
    let inst = sfz::parse_str("<region> sample=a.wav", "a.sfz").unwrap();
    let r = &inst.regions[0];
    assert_eq!((r.lokey, r.hikey, r.lovel, r.hivel), (0, 127, 0, 127));
    assert_eq!(r.pitch_keycenter, 60);
    assert_eq!(r.amp_veltrack, 100.0);
    assert_eq!(r.ampeg.sustain, 1.0);
    assert_eq!(r.ampeg.attack, 0.0);
    assert_eq!(r.effective_loop_mode(), LoopMode::NoLoop);
    assert_eq!((r.seq_length, r.seq_position), (1, 1));
    // Loop points without a mode loop continuously.
    let inst = sfz::parse_str("<region> sample=a.wav loop_start=10 loop_end=99", "a.sfz").unwrap();
    assert_eq!(inst.regions[0].effective_loop_mode(), LoopMode::Continuous);
}

#[test]
fn group_resets_and_global_persists() {
    let text = "<global> volume=-3
<group> lovel=1 hivel=60
<region> sample=a.wav
<group> transpose=2
<region> sample=b.wav
<master> pan=50
<region> sample=c.wav
<global>
<region> sample=d.wav";
    let r = sfz::parse_str(text, "x.sfz").unwrap().regions;
    assert_eq!((r[0].volume, r[0].hivel), (-3.0, 60));
    assert_eq!((r[1].volume, r[1].hivel, r[1].transpose), (-3.0, 127, 2));
    // <master> clears the group.
    assert_eq!((r[2].pan, r[2].transpose), (50.0, 0));
    // A new <global> clears everything.
    assert_eq!((r[3].volume, r[3].pan), (0.0, 0.0));
}

#[test]
fn defines_longest_name_first_and_comments() {
    let text = "#define $V 1
#define $V10 100
<region> sample=a.wav // trailing comment
 hivel=$V10 lovel=$V /* inline */ tune=5";
    let r = &sfz::parse_str(text, "x.sfz").unwrap().regions[0];
    assert_eq!((r.lovel, r.hivel, r.tune), (1, 100, 5.0));
}

#[test]
fn control_offsets_and_default_path() {
    let text = "<control> default_path=samples\\piano\\ note_offset=2 octave_offset=-1
<region> sample=c4.wav key=c4
<region> sample=../x.wav lokey=10 hikey=20";
    let r = sfz::parse_str(text, "pack/inst/p.sfz").unwrap().regions;
    assert_eq!(r[0].sample, file("pack/inst/samples/piano/c4.wav"));
    // 2 - 12 = -10 keys.
    assert_eq!((r[0].lokey, r[0].pitch_keycenter), (50, 50));
    assert_eq!(r[1].sample, file("pack/inst/samples/x.wav"));
    assert_eq!((r[1].lokey, r[1].hikey), (0, 10));
}

#[test]
fn unsupported_opcodes_warn_once_with_line() {
    let text = "<region> sample=a.wav amp_velcurve_127=1 foo=2
<region> sample=b.wav foo=3
<effect> type=reverb";
    let inst = sfz::parse_str(text, "w.sfz").unwrap();
    assert_eq!(inst.regions.len(), 2);
    assert_eq!(inst.warnings.len(), 3, "{:?}", inst.warnings);
    assert!(inst.warnings[0].starts_with("w.sfz:1: unsupported opcode `amp_velcurve_127=1`"));
    assert!(
        inst.warnings
            .iter()
            .any(|w| w.starts_with("w.sfz:1:") && w.contains("foo"))
    );
    assert!(
        inst.warnings
            .iter()
            .any(|w| w.starts_with("w.sfz:3: <effect>"))
    );
}

fn err(text: &str) -> SfzError {
    sfz::parse_str(text, "e.sfz").unwrap_err()
}

#[test]
fn errors_carry_file_and_line() {
    let e = err("<region> sample=a.wav\n\n<regoin> sample=b.wav");
    assert_eq!((e.file.as_str(), e.line), ("e.sfz", 3));
    assert!(e.msg.contains("unknown header <regoin>"), "{e}");
    assert_eq!(e.to_string(), "e.sfz:3: unknown header <regoin>");

    let e = err("<region>\n sample=a.wav lokey=c4 hikey=x9");
    assert_eq!(e.line, 2);
    assert!(e.msg.contains("hikey"), "{e}");

    let e = err("<region> sample=a.wav\n volume=loud");
    assert_eq!(e.line, 2);
    assert!(
        e.msg.contains("volume") && e.msg.contains("not a valid number"),
        "{e}"
    );

    let e = err("<region> lovel sample=a.wav");
    assert!(
        e.msg.contains("expected `opcode=value`, found `lovel`"),
        "{e}"
    );

    let e = err("<region> sample=a.wav lokey=1 2");
    assert!(e.msg.contains("unexpected `2`"), "{e}");

    let e = err("// ok\n<region> lokey=1");
    assert_eq!(e.line, 2);
    assert!(e.msg.contains("region has no sample"), "{e}");

    let e = err("<region> sample=a.wav lokey=70 hikey=60");
    assert!(e.msg.contains("lokey 70 > hikey 60"), "{e}");

    let e = err("<region> sample=a.wav /* never closed\n\n");
    assert_eq!(e.line, 1);
    assert!(e.msg.contains("unterminated"), "{e}");

    let e = err("<region> sample=../../etc/passwd");
    assert!(e.msg.contains("leaves the pack"), "{e}");

    let e = err("<region> sample=a.wav loop_mode=forever");
    assert!(e.msg.contains("unknown loop_mode"), "{e}");

    let e = err("lokey=1 <region> sample=a.wav");
    assert!(e.msg.contains("before any header"), "{e}");

    let e = err("<region> sample=*sine");
    assert!(e.msg.contains("not supported"), "{e}");

    let e = err("<region sample=a.wav");
    assert!(e.msg.contains("missing `>`"), "{e}");

    let e = err("#define NAME 1");
    assert!(e.msg.contains("must start with $"), "{e}");
}

#[test]
fn include_errors() {
    let mut files: BTreeMap<String, String> = BTreeMap::new();
    files.insert(
        "p/a.sfz".into(),
        "<region> sample=x.wav\n#include \"b.sfz\"".into(),
    );
    files.insert("p/b.sfz".into(), "#include \"a.sfz\"".into());
    files.insert("p/c.sfz".into(), "#include \"missing.sfz\"".into());
    files.insert("p/d.sfz".into(), "#include missing.sfz".into());
    let mut read = |p: &str| files.get(p).cloned().ok_or_else(|| format!("no `{p}`"));
    let e = sfz::parse_file("p/a.sfz", &mut read).unwrap_err();
    assert!(e.msg.contains("nested too deeply"), "{e}");
    let e = sfz::parse_file("p/c.sfz", &mut read).unwrap_err();
    assert_eq!((e.file.as_str(), e.line), ("p/c.sfz", 1));
    assert!(e.msg.contains("cannot include `missing.sfz`"), "{e}");
    let e = sfz::parse_file("p/d.sfz", &mut read).unwrap_err();
    assert!(e.msg.contains("quoted"), "{e}");
    // Errors inside an included file name that file.
    files.insert("p/e.sfz".into(), "#include \"f.sfz\"".into());
    files.insert("p/f.sfz".into(), "\n<region> sample=a.wav tune=x".into());
    let mut read = |p: &str| files.get(p).cloned().ok_or_else(|| format!("no `{p}`"));
    let e = sfz::parse_file("p/e.sfz", &mut read).unwrap_err();
    assert_eq!((e.file.as_str(), e.line), ("p/f.sfz", 2));
}

#[test]
fn parses_the_installed_upright_piano_kw_sfz() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../packs/upright-piano-kw/UprightPianoKW-20220221.sfz");
    let Ok(text) = std::fs::read_to_string(&path) else {
        println!("SKIPPED: {} not installed", path.display());
        return;
    };
    let inst = sfz::parse_str(&text, "upright-piano-kw/UprightPianoKW-20220221.sfz").unwrap();
    assert_eq!(inst.regions.len(), 66);
    assert!(inst.warnings.is_empty(), "{:?}", inst.warnings);
    let lpf = inst.regions.iter().filter(|r| r.filter.is_some()).count();
    assert_eq!(lpf, 3);
}
