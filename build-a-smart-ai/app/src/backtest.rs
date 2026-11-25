//! `--backtest`: lets the app plan the transitions of every training DJ mix on its own
//! (no hints) and compares them with what the real DJ did.
use crate::dj::{self, MixOpts, Settings};
use crate::tracks::{self, SR};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

struct Seg {
    song: usize,
    mix_start: f64,
    mix_end: f64,
    song_start: f64,
    song_end: f64,
}

fn songs_list(dir: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir).unwrap().filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| p.extension().map(|e| e == "mp3").unwrap_or(false)).collect();
    v.retain(|p| !p.to_string_lossy().contains("Signal Addition"));
    v.sort();
    v
}

fn read_segments(root: &Path) -> Vec<Seg> {
    let t = std::fs::read_to_string(root.join("results/segments.tsv")).unwrap_or_default();
    t.lines()
        .skip(1)
        .filter_map(|l| {
            let p: Vec<&str> = l.split('\t').collect();
            Some(Seg { song: p.first()?.parse().ok()?, mix_start: p.get(2)?.parse().ok()?, mix_end: p.get(3)?.parse().ok()?, song_start: p.get(4)?.parse().ok()?, song_end: p.get(5)?.parse().ok()? })
        })
        .collect()
}

fn median(v: &mut Vec<f64>) -> f64 {
    if v.is_empty() {
        return f64::NAN;
    }
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    v[v.len() / 2]
}

pub fn run(base: &Path) {
    let sr = SR as f64;
    let mut out = String::new();
    macro_rules! say {
        ($($t:tt)*) => {{ let l = format!($($t)*); println!("{}", l); out.push_str(&l); out.push('\n'); }};
    }
    let (mut all_out, mut all_in, mut base_out, mut base_in, mut all_len, mut ranks): (Vec<f64>, Vec<f64>, Vec<f64>, Vec<f64>, Vec<(f64, f64)>, Vec<(usize, usize)>) = (vec![], vec![], vec![], vec![], vec![], vec![]);
    let set = Settings::new();
    for n in 1..=11 {
        let root = base.join(format!("training {}", n));
        let segs = read_segments(&root);
        if segs.is_empty() {
            say!("training {}: no data", n);
            continue;
        }
        let dir = if root.join("songs").is_dir() { root.join("songs") } else { root.join("tracks") };
        let files = songs_list(&dir);
        let prog = std::sync::Arc::new(Mutex::new(tracks::Progress::default()));
        tracks::start_scan(dir.clone(), prog.clone());
        while !prog.lock().unwrap().finished {
            std::thread::sleep(std::time::Duration::from_millis(300));
        }
        let mut tv = prog.lock().unwrap().result.take().unwrap_or_default();
        tracks::finalize(&mut tv);
        // song index (trainer order) -> track
        let find = |i: usize| -> Option<usize> {
            let f = files.get(i)?.file_name()?.to_owned();
            tv.iter().position(|t| t.path.as_ref().and_then(|p| p.file_name()) == Some(f.as_os_str()))
        };
        let mut main: Vec<(usize, &Seg)> = Vec::new();
        let mut used: Vec<usize> = segs.iter().map(|s| s.song).collect();
        used.sort();
        used.dedup();
        for si in used {
            let Some(best) = segs.iter().filter(|s| s.song == si).max_by(|a, b| (a.mix_end - a.mix_start).partial_cmp(&(b.mix_end - b.mix_start)).unwrap()) else { continue };
            if let Some(ti) = find(si) {
                main.push((ti, best));
            }
        }
        main.sort_by(|a, b| a.1.mix_start.partial_cmp(&b.1.mix_start).unwrap());
        let prof = dj::Profile::new(tv.len());
        say!("\n== training {}: {} songs placed ==", n, main.len());
        for k in 0..main.len().saturating_sub(1) {
            let ((ia, sa), (ib, sb)) = (main[k], main[k + 1]);
            if sb.mix_start - sa.mix_end >= 80.0 {
                continue;
            }
            let (ta, tb) = (&tv[ia], &tv[ib]);
            // would the app choose the same next song among the ones still to come?
            let eff = ta.a.bpm;
            let mut sc: Vec<(usize, f32)> = main[k + 1..].iter().map(|&(j, _)| (j, dj::score_candidate(&tv[j], Some(ta), Some(eff), &set, &prof, &[]).score)).collect();
            sc.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
            let rank = sc.iter().position(|x| x.0 == ib).unwrap_or(0);
            ranks.push((rank, sc.len()));
            let opts = MixOpts { after_melody: true, club: std::env::var("SMOOTH").is_err(), ..Default::default() };
            let Some(m) = dj::choose_mix(ta, tb, 0.0, 1.0, 32, false, opts, None) else {
                say!("{:<28} -> {:<28} app found no transition", ta.title.chars().take(28).collect::<String>(), tb.title.chars().take(28).collect::<String>());
                continue;
            };
            let (app_out, app_in) = (m.out_pos / sr, m.in_pos / sr);
            let (e_out, e_in) = (app_out - sa.song_end, app_in - sb.song_start);
            let bpm = tb.a.bpm.max(60.0) as f64;
            let dj_len = (sa.mix_end - sb.mix_start).max(0.0);
            let app_len = m.beats as f64 * 60.0 / bpm;
            all_out.push(e_out.abs());
            all_in.push(e_in.abs());
            all_len.push((app_len, dj_len));
            // baseline: the DJ's point versus every candidate point the app could have picked
            let mut bo: Vec<f64> = dj::top_out_points(ta, 0.0, 400).iter().map(|p| (p.pos / sr - sa.song_end).abs()).collect();
            let mut bi: Vec<f64> = dj::top_in_points(tb, 400).iter().map(|p| (p.pos / sr - sb.song_start).abs()).collect();
            base_out.push(median(&mut bo));
            base_in.push(median(&mut bi));
            say!(
                "{:<26} -> {:<26} rank {}/{} | out app {:>5.0}s dj {:>5.0}s ({:+.0}) | in app {:>5.0}s dj {:>5.0}s ({:+.0}) | len app {:>4.1}s dj {:>4.1}s",
                ta.title.chars().take(26).collect::<String>(),
                tb.title.chars().take(26).collect::<String>(),
                rank + 1,
                sc.len(),
                app_out,
                sa.song_end,
                e_out,
                app_in,
                sb.song_start,
                e_in,
                app_len,
                dj_len
            );
        }
    }
    let n = all_out.len();
    let within = |v: &Vec<f64>, s: f64| v.iter().filter(|&&x| x <= s).count();
    say!("\n================ SUMMARY over {} transitions ================", n);
    say!("exit point: median error {:.0}s (a random candidate point: {:.0}s) | within 8s {}/{} | within 20s {}/{}", median(&mut all_out.clone()), median(&mut base_out.clone()), within(&all_out, 8.0), n, within(&all_out, 20.0), n);
    say!("entry point: median error {:.0}s (a random candidate point: {:.0}s) | within 8s {}/{} | within 20s {}/{}", median(&mut all_in.clone()), median(&mut base_in.clone()), within(&all_in, 8.0), n, within(&all_in, 20.0), n);
    let (la, ld): (Vec<f64>, Vec<f64>) = all_len.iter().cloned().unzip();
    say!("blend length: app median {:.1}s, DJ median {:.1}s", median(&mut la.clone()), median(&mut ld.clone()));
    let top1 = ranks.iter().filter(|r| r.0 == 0).count();
    let top3 = ranks.iter().filter(|r| r.0 < 3).count();
    let rnd1: f64 = ranks.iter().map(|r| 1.0 / r.1 as f64).sum();
    say!("next song: app's #1 pick matched the DJ {}/{} (chance would be {:.1}), in its top 3: {}/{}", top1, ranks.len(), rnd1, top3, ranks.len());
    let _ = std::fs::write(base.join("backtest.txt"), out);
}
