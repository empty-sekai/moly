//! Acceptance harness: drives the product's own laws, loaders and state
//! updates on input grids whose expected values were produced by the game's
//! own code (executed outside this repository), and writes what the product
//! did. The comparison against the expected values is made outside the
//! product; nothing here pins the product's current behaviour.
//!
//! Every driver is `#[ignore]` and reads `MOLY_NPC_HARNESS_DIR`: cases from
//! `<dir>/in/<name>.json`, results to `<dir>/out/<name>.json`. The state and
//! idle-script drivers live beside the private code they drive
//! (`npc_state`, `alone_action_runtime`).

use serde_json::{json, Value};

pub(crate) fn case(name: &str) -> Value {
    let dir = std::env::var("MOLY_NPC_HARNESS_DIR").expect("MOLY_NPC_HARNESS_DIR");
    let text = std::fs::read_to_string(format!("{dir}/in/{name}.json"))
        .unwrap_or_else(|err| panic!("harness case {name}: {err}"));
    serde_json::from_str(&text).unwrap_or_else(|err| panic!("harness case {name}: {err}"))
}

pub(crate) fn emit(name: &str, value: &Value) {
    let dir = std::env::var("MOLY_NPC_HARNESS_DIR").expect("MOLY_NPC_HARNESS_DIR");
    std::fs::create_dir_all(format!("{dir}/out")).expect("harness out dir");
    std::fs::write(
        format!("{dir}/out/{name}.json"),
        serde_json::to_string(value).expect("harness result"),
    )
    .expect("write harness result");
}

fn f32_of(value: &Value) -> f32 {
    value.as_f64().expect("number") as f32
}

mod decision {
    use super::*;
    use moly_law::objective::{
        decide, interrupt_dispatch, InterruptMarker, LadderView, ObjectiveType, TalkType,
    };
    use moly_law::talk::select::{
        select_objective, LotteryPercents, Objective, PercentDraw, TalkLane,
    };

    struct Queue {
        values: [u32; 3],
        taken: usize,
    }

    impl PercentDraw for Queue {
        fn draw_percent(&mut self) -> u32 {
            let value = self.values.get(self.taken).copied().unwrap_or(0);
            self.taken += 1;
            value
        }
    }

    fn letter(objective: Objective) -> char {
        match objective {
            Objective::Talk(TalkLane::YetUnreadFixtureTalk) => 'A',
            Objective::Talk(TalkLane::GeneralTalk) => 'D',
            Objective::NoneTalk => 'E',
            Objective::Talk(TalkLane::AlreadyReadTalkFixtureTalk) => 'F',
            Objective::Talk(TalkLane::FixtureCommonTalk) => 'G',
        }
    }

    fn percents(p: &Value) -> LotteryPercents {
        LotteryPercents {
            fixture_talk: f32_of(&p["142"]),
            already_read_fixture_talk: f32_of(&p["149"]),
            none_talk_fixture_action: f32_of(&p["143"]),
            already_read_when_has_not_read: f32_of(&p["148"]),
        }
    }

    /// The objective ladder over every draw triple, composed from the
    /// source's two executed ladders, and the decision rows.
    #[test]
    #[ignore = "requires MOLY_NPC_HARNESS_DIR"]
    fn a01_decision_map() {
        let input = case("a01");
        let mut sets = serde_json::Map::new();
        for (name, set) in input["sets"].as_object().expect("sets") {
            let p = percents(&set["p"]);
            let so: Vec<char> = set["so"].as_str().expect("so").chars().collect();
            let so_draws: Vec<u64> = set["so_draws"].as_array().unwrap().iter().map(|v| v.as_u64().unwrap()).collect();
            let sft: Vec<char> = set["sft"].as_str().expect("sft").chars().collect();
            let sft_draws: Vec<u64> = set["sft_draws"].as_array().unwrap().iter().map(|v| v.as_u64().unwrap()).collect();
            let (mut compared, mut outcome_mismatch, mut draw_mismatch) = (0u64, 0u64, 0u64);
            let mut examples = Vec::new();
            let mut tally_product = std::collections::BTreeMap::<char, u64>::new();
            let mut tally_source = std::collections::BTreeMap::<char, u64>::new();
            for unread in 0..2usize {
                for d in 0..100u32 {
                    let so_letter = so[unread * 100 + d as usize];
                    for d1 in 0..100u32 {
                        for d2 in 0..100u32 {
                            let (want, want_draws) = if so_letter == 'S' {
                                let i = (d1 * 100 + d2) as usize;
                                (sft[i], so_draws[unread * 100 + d as usize] + sft_draws[i])
                            } else {
                                (so_letter, so_draws[unread * 100 + d as usize])
                            };
                            let mut queue = Queue { values: [d, d1, d2], taken: 0 };
                            let got = letter(select_objective(&p, unread == 1, &mut queue));
                            compared += 1;
                            *tally_product.entry(got).or_default() += 1;
                            *tally_source.entry(want).or_default() += 1;
                            let bad_outcome = got != want;
                            let bad_draws = queue.taken as u64 != want_draws;
                            outcome_mismatch += bad_outcome as u64;
                            draw_mismatch += bad_draws as u64;
                            if (bad_outcome || bad_draws) && examples.len() < 20 {
                                examples.push(json!({"unread": unread, "d": d, "d1": d1, "d2": d2, "want": want.to_string(), "got": got.to_string(), "want_draws": want_draws, "got_draws": queue.taken}));
                            }
                        }
                    }
                }
            }
            sets.insert(
                name.clone(),
                json!({"compared": compared, "outcome_mismatch": outcome_mismatch, "draw_mismatch": draw_mismatch, "examples": examples,
                       "tally_product": tally_product.iter().map(|(k, v)| (k.to_string(), json!(v))).collect::<serde_json::Map<_, _>>(),
                       "tally_source": tally_source.iter().map(|(k, v)| (k.to_string(), json!(v))).collect::<serde_json::Map<_, _>>()}),
            );
        }
        let master = percents(&input["sets"]["master"]["p"]);
        let mut rows = Vec::new();
        for row in input["decide"].as_array().expect("decide") {
            let view = LadderView {
                talk_type: row["talk"].as_u64().map(|t| TalkType::from_discriminant(t as u8).expect("talk type")),
                photo_shot: row["photo"].as_u64() == Some(1),
                interrupt: row["marker"].as_array().map(|m| InterruptMarker {
                    marker_type: m[0].as_i64().unwrap() as i32,
                    can_interrupt: m[1].as_i64() == Some(1),
                }),
                current: row["current"].as_u64().map(|c| ObjectiveType::from_discriminant(c as u8).expect("objective type")),
                first_talk_complete: row["first"].as_u64() == Some(1),
            };
            let mut queue = Queue { values: [0, 0, 0], taken: 0 };
            let decision = decide(&view, &master, false, &mut queue);
            rows.push(json!({"decision": format!("{decision:?}"), "can_cancel": decision.can_cancel(), "draws": queue.taken}));
        }
        let interrupt: Vec<Value> = input["interrupt"]
            .as_array()
            .expect("interrupt")
            .iter()
            .map(|row| json!(format!("{:?}", interrupt_dispatch(row["marker_type"].as_i64().unwrap() as i32))))
            .collect();
        emit("a01", &json!({"sets": sets, "decide": rows, "interrupt": interrupt}));
    }
}

mod member_count {
    use super::*;
    use moly_law::talk::select::{
        lottery_member_count, lottery_talk_id, Candidate, LotteryWeights, WeightedDraw,
    };
    use moly_law::talk::{EnumerablePick, NetRandom};

    struct Fixed {
        draw: Option<f32>,
        total: Option<f32>,
        calls: usize,
    }

    impl WeightedDraw for Fixed {
        fn draw_weight(&mut self, total: f32) -> f32 {
            self.calls += 1;
            self.total = Some(total);
            // "sum": the draw equal to the range maximum handed to the engine.
            self.draw.unwrap_or(total)
        }
    }

    struct Seeded {
        seed: i32,
        calls: usize,
    }

    impl EnumerablePick for Seeded {
        fn pick(&mut self, count: usize) -> Option<usize> {
            self.calls += 1;
            let mut random = NetRandom::new(self.seed);
            if count == 0 {
                return None;
            }
            Some(random.next_below(count as i32) as usize)
        }
    }

    fn weights(name: &str) -> LotteryWeights {
        let w = match name {
            "master" => [40.0, 20.0, 20.0, 20.0],
            "asym" => [10.0, 20.0, 30.0, 40.0],
            "nonint" => [0.1, 0.2, 0.3, 16777216.0],
            "zero3" => [40.0, 20.0, 0.0, 20.0],
            "allzero" => [0.0, 0.0, 0.0, 0.0],
            "scan_below_sum" => [5.4, 2.5, 12.5, 0.8],
            "scan_above_sum" => [19.6, 11.3, 12.7, 14.2],
            other => panic!("weight set {other}"),
        };
        LotteryWeights { talk1: w[0], talk2: w[1], talk3: w[2], talk4: w[3] }
    }

    /// The member-count lottery on the executed cases, then the whole talk-id
    /// lottery over pools holding talks of the enabled member counts.
    #[test]
    #[ignore = "requires MOLY_NPC_HARNESS_DIR"]
    fn a015_member_count_and_pick() {
        let input = case("a015");
        const BUCKET: [usize; 4] = [3, 5, 2, 7];
        const SEEDS: [i32; 5] = [0, 1, 2, 42, 12345];
        let mut rows = Vec::new();
        for row in input["membercount"].as_array().expect("membercount") {
            let w = weights(row["weights"].as_str().unwrap());
            let counts: Vec<i32> = row["counts"].as_array().unwrap().iter().map(|v| v.as_i64().unwrap() as i32).collect();
            let draw = row["draw"].as_f64().map(|d| d as f32);
            let mut fixed = Fixed { draw, total: None, calls: 0 };
            let count = lottery_member_count(&counts, &w, &mut fixed);
            let mut picks = Vec::new();
            let plain = !counts.is_empty()
                && counts.iter().all(|c| (1..=4).contains(c))
                && counts.windows(2).all(|p| p[0] < p[1]);
            if plain {
                let mut candidates = Vec::new();
                for &c in &counts {
                    for i in 0..BUCKET[(c - 1) as usize] {
                        candidates.push(Candidate { talk_id: 1000 * c + i as i32, member_count: c, passes_gates: true });
                    }
                }
                for seed in SEEDS {
                    let mut fixed = Fixed { draw, total: None, calls: 0 };
                    let mut pick = Seeded { seed, calls: 0 };
                    let outcome = lottery_talk_id(&candidates, &w, &mut fixed, &mut pick);
                    picks.push(match outcome {
                        Ok(o) => json!({"seed": seed, "talk_id": o.talk_id, "member_count": o.member_count, "weighted_calls": fixed.calls, "pick_calls": pick.calls}),
                        Err(fault) => json!({"seed": seed, "fault": format!("{fault:?}"), "weighted_calls": fixed.calls, "pick_calls": pick.calls}),
                    });
                }
            }
            rows.push(json!({"member_count": count, "draw_calls": fixed.calls, "total_bits": fixed.total.map(f32::to_bits), "picks": picks}));
        }
        // A pool whose talks all fail the gates: talk id 0 with no draw.
        let gated = [Candidate { talk_id: 5, member_count: 1, passes_gates: false }];
        let mut fixed = Fixed { draw: Some(0.0), total: None, calls: 0 };
        let mut pick = Seeded { seed: 0, calls: 0 };
        let empty = lottery_talk_id(&gated, &weights("master"), &mut fixed, &mut pick);
        emit("a015", &json!({"rows": rows, "buckets": BUCKET, "seeds": SEEDS,
                             "gated_pool": format!("{empty:?}"), "gated_pool_calls": [fixed.calls, pick.calls]}));
    }
}

mod rest {
    use super::*;
    use moly_law::frame_time::{FrameClock, TimeSettings};
    use moly_law::objective::{rest_delay_milliseconds, rest_objective_milliseconds, DelayPromise};

    fn times(kind: &str, n: usize) -> Vec<f64> {
        match kind {
            "60Hz" => (1..=n).map(|k| k as f64 / 60.0).collect(),
            "30Hz" => (1..=n).map(|k| k as f64 / 30.0).collect(),
            "4Hz" => (1..=n).map(|k| k as f64 * 0.25).collect(),
            _ => {
                let long: f64 = kind
                    .strip_prefix("60Hz+")
                    .and_then(|s| s.strip_suffix('s'))
                    .and_then(|s| s.parse().ok())
                    .unwrap_or_else(|| panic!("frame kind {kind}"));
                let mut t = 0.0;
                (1..=n)
                    .map(|k| {
                        t += if k == 120 { long } else { 1.0 / 60.0 };
                        t
                    })
                    .collect()
            }
        }
    }

    /// Frames from creation (at frame 100) to completion of the product's
    /// delay on the product's engine clock.
    fn complete_after(ms: i32, kind: &str, cap: f32) -> Option<usize> {
        let settings = TimeSettings { maximum_allowed_timestep: cap, time_scale: 1.0 };
        let mut clock = FrameClock::reset_with_first_delta(settings, 0.0);
        let deltas: Vec<f32> = times(kind, 2000)
            .into_iter()
            .map(|t| {
                clock.update(t);
                clock.delta()
            })
            .collect();
        let mut delay = DelayPromise::from_milliseconds(ms).ok()?;
        let created = 100usize;
        (created + 1..deltas.len()).find(|frame| delay.advance(deltas[frame - 1])).map(|frame| frame - created)
    }

    #[test]
    #[ignore = "requires MOLY_NPC_HARNESS_DIR"]
    fn a03_rest_delay() {
        let input = case("a03");
        let rest_ms: Vec<Value> = input["rest_ms"]["rows"]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| {
                let bits = u32::from_str_radix(row["f32_bits"].as_str().unwrap().trim_start_matches("0x"), 16).unwrap();
                json!({"bits": bits, "ms": rest_objective_milliseconds(f32::from_bits(bits))})
            })
            .collect();
        let route: Vec<Value> = input["route_rest"]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| {
                let seconds = match &row["pause_seconds"] {
                    Value::String(s) => s.parse::<f32>().unwrap_or_else(|_| panic!("pause seconds {s}")),
                    v => f32_of(v),
                };
                json!({"ms": rest_delay_milliseconds(seconds)})
            })
            .collect();
        let delay: Vec<Value> = input["delay"]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| {
                let ms = row["ms"].as_i64().unwrap() as i32;
                let promise = DelayPromise::from_milliseconds(ms).ok();
                json!({"delay_bits": promise.map(|p| p.delay().to_bits()),
                       "after": complete_after(ms, row["frames"].as_str().unwrap(), f32::from_bits(0x3eaa_aaab))})
            })
            .collect();
        let arms: Vec<Value> = input["arms"]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| {
                let cap = match row["cap"].as_str().unwrap() {
                    "time-manager 1/3 s" => f32::from_bits(0x3eaa_aaab),
                    "unclamped 10 s" => 10.0,
                    "Bevy default 0.25 s" => 0.25,
                    other => panic!("cap {other}"),
                };
                json!({"after": complete_after(row["ms"].as_i64().unwrap() as i32, row["frames"].as_str().unwrap(), cap)})
            })
            .collect();
        emit("a03", &json!({"rest_ms": rest_ms, "route_rest": route, "delay": delay, "arms": arms}));
    }
}

mod greeting {
    use super::*;
    use moly_law::tweet as tweet_law;

    struct Index {
        value: usize,
        len: Option<usize>,
    }

    impl tweet_law::UniformDraw for Index {
        fn draw(&mut self, len: usize) -> usize {
            self.len = Some(len);
            self.value
        }
    }

    /// The greeting candidates the product's pick draws from, per unit,
    /// visit count and phenomenon, on the tables the product loads.
    #[test]
    #[ignore = "requires MOLY_NPC_HARNESS_DIR and MOLY_ASSET_ROOT"]
    fn a09_greeting_candidates() {
        let root = std::env::var("MOLY_ASSET_ROOT").expect("MOLY_ASSET_ROOT");
        let text = std::fs::read_to_string(format!("{root}/tweet-tables.json")).expect("tweet-tables.json");
        let (wrt, greetings, conditions) = crate::npc_tweet::parse_greeting_tables(&text);
        let input = case("a09");
        let mut rows = Vec::new();
        for query in input["queries"].as_array().unwrap() {
            let unit = query["unit"].as_i64().unwrap() as i32;
            let visit = query["visit"].as_i64().unwrap() as i32;
            let phenomena = query["phenomena"].as_i64().unwrap() as i32;
            let mut probe = Index { value: 0, len: None };
            let first = tweet_law::pick_greeting(&greetings, &wrt, &conditions, unit, visit, phenomena, &mut probe);
            let candidates: Vec<Value> = match (first, probe.len) {
                (Some(_), Some(len)) => (0..len)
                    .map(|i| {
                        let mut draw = Index { value: i, len: None };
                        let g = tweet_law::pick_greeting(&greetings, &wrt, &conditions, unit, visit, phenomena, &mut draw)
                            .expect("same pool");
                        json!([g.id, tweet_law::greeting_tweet_id(g, &wrt)])
                    })
                    .collect(),
                _ => Vec::new(),
            };
            rows.push(json!({"candidates": candidates, "drew": probe.len.is_some()}));
        }
        emit("a09", &Value::Array(rows));
    }
}

mod target {
    use super::*;
    use moly_law::objective::social::nearby_spots;
    use moly_law::objective::wander::{ring_filter, uses_in_room_move_range};
    use moly_law::objective::SurfaceProbe;

    /// Surface answers by call order: the listed navigability calls miss.
    struct Scripted {
        misses: Vec<usize>,
        calls: usize,
        sources: Vec<[f32; 3]>,
    }

    impl SurfaceProbe for Scripted {
        fn sample(&mut self, target: [f32; 3], _tolerance: f32) -> Option<[f32; 3]> {
            let index = self.calls;
            self.calls += 1;
            (!self.misses.contains(&index)).then_some(target)
        }
        fn has_path(&mut self, source: [f32; 3], _target: [f32; 3]) -> bool {
            self.sources.push(source);
            true
        }
    }

    fn vec3(value: &Value) -> [f32; 3] {
        let a = value.as_array().expect("vector");
        [f32_of(&a[0]), f32_of(&a[1]), f32_of(&a[2])]
    }

    /// The general-talk target kernels: the wander ring predicate, the move
    /// range selector and the social spot sequence.
    #[test]
    #[ignore = "requires MOLY_NPC_HARNESS_DIR"]
    fn a05_target_kernels() {
        let input = case("a05");
        let ring: Vec<Value> = input["ring"]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| {
                let s = row["start"].as_array().unwrap();
                let c = row["cand"].as_array().unwrap();
                json!(ring_filter(
                    (s[0].as_i64().unwrap() as i32, s[2].as_i64().unwrap() as i32),
                    (c[0].as_i64().unwrap() as i32, c[2].as_i64().unwrap() as i32),
                    row["min"].as_i64().unwrap() as i32,
                    row["max"].as_i64().unwrap() as i32,
                ))
            })
            .collect();
        let range: Vec<Value> = input["move_range"]
            .as_array()
            .unwrap()
            .iter()
            .map(|site| json!(uses_in_room_move_range(site.as_i64().unwrap() as i32)))
            .collect();
        let spots: Vec<Value> = input["spots"]
            .as_array()
            .unwrap()
            .iter()
            .map(|scenario| {
                let npcs: Vec<[f32; 3]> = scenario["npcs"].as_array().unwrap().iter().map(vec3).collect();
                let misses = scenario["not_nav"].as_array().unwrap().iter().map(|v| v.as_u64().unwrap() as usize).collect();
                let mut probe = Scripted { misses, calls: 0, sources: Vec::new() };
                let out = nearby_spots(vec3(&scenario["tgt"]), vec3(&scenario["src"]), &npcs, &mut probe);
                json!({
                    "spots_bits": out.iter().map(|p| p.map(f32::to_bits)).collect::<Vec<_>>(),
                    "nav_calls": probe.calls,
                    "nav_sources_bits": probe.sources.iter().map(|p| p.map(f32::to_bits)).collect::<Vec<_>>(),
                })
            })
            .collect();
        // The product's world-to-cell conversion on the executed conversion
        // inputs (x and z; the product's cell has no height axis).
        let to_grid: Vec<Value> = input["to_grid"]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| {
                let x: f32 = row[0].as_str().unwrap().parse().unwrap();
                let z: f32 = row[2].as_str().unwrap().parse().unwrap();
                let (cx, cz) = crate::npc_objective::cell_of(x, z);
                json!([cx, cz])
            })
            .collect();
        emit(
            "a05",
            &json!({"ring": ring, "move_range": range, "spots": spots, "to_grid": to_grid}),
        );
    }
}
