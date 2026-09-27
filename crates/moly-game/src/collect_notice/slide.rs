//! `SlideNoticeContoller` and `SlideNoticeDriver` as the notice layer runs
//! them for the collect-item cells, and the layer's pool of view data.
//!
//! The source, read from the compiled methods:
//! - `ScreenLayerMysekaiNotice.SetupCollectItem`: a `ResourcePool` of 64 view
//!   data objects, then `Initialize(5, 8, collectItemCellPrefab,
//!   collectItemRoot, SlideInLeft)`. `ResourcePool.GetResource` is a ring: it
//!   returns entry `index` and advances `index` modulo 64, so the 65th notice
//!   reuses the first object even while that object still waits in a queue.
//! - `Initialize`: one cell per driver, instantiated under the root and named
//!   the prefab's name plus the index, `Sleep()`, `anchoredPosition = (x,
//!   -rect.height * index)`; then `new SlideNoticeDriver(index, 8, cell, rect,
//!   actionType)`, whose start position is the anchored position plus
//!   (-600, 0) for SlideInLeft (+600 for SlideInRight). The 8 is only the
//!   initial capacity of the driver's `Queue`: nothing is ever dropped.
//! - `Enqueue(data)`: when no driver plays the driver index returns to 0;
//!   the data goes to `drivers[index]`, then the index advances modulo 5.
//! - `SlideNoticeDriver.Enqueue`: the data joins the queue and, when the
//!   driver's queue task is not pending, `PlayQueue` starts: while the queue
//!   holds data, `Peek`, `cell.Setup(data)`, await `Play(data)`, `Dequeue`.
//! - `Play`: `Wakeup`, `Open`, `anchoredPosition = startPos`,
//!   `DOAnchorPosX(0, 0.2)` with OutCubic (not awaited), then `time = 0` and
//!   `while (time < data.NoticeDuration) { time += Time.deltaTime; await
//!   UniTask.Yield(); }` (the duration is read from the data each pass),
//!   `Close`, `DOAnchorPosX(startPos.x, 0.2)` with OutCubic awaited, `Sleep`.
//!   `Open` and `Close` of the collect-item cell are empty.
//!
//! Frame order, not fixed by the source (it is the player loop's and the
//! callers' order): calls first, then the yield continuations (UniTask's
//! Update timing), then the tween step, in which a tween created earlier in
//! the frame takes its first step. A tween created by a completion inside
//! the tween step (the next notice of a driver) takes its first step on the
//! next frame.

use std::collections::VecDeque;

use moly_law::ui::dotween::{Ease, FloatTween};

/// `Initialize`'s parallel count.
pub(crate) const PARALLEL: usize = 5;
/// `ResourcePool<HarvestCollectItemNoticeViewData>(64)`.
pub(crate) const POOL: usize = 64;
/// The SlideInLeft start offset of the driver.
pub(crate) const SLIDE_IN_LEFT: f32 = -600.0;
/// Both `DOAnchorPosX` durations of `Play`.
pub(crate) const SLIDE_DURATION: f32 = 0.2;
/// `DOAnchorPosX`'s target x of the slide-in.
const SHOWN_X: f32 = 0.0;

/// What a driver does to its cell, in order.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum CellCall {
    /// `cell.Setup(data)` with the pool slot's data.
    Setup { slot: usize },
    /// `Wakeup` (the cell's object becomes active).
    Wakeup,
    /// `Sleep` (the cell's object becomes inactive).
    Sleep,
    /// `anchoredPosition.x` written (by `Play` or by a tween step).
    X(f32),
}

#[derive(Debug, Clone, PartialEq)]
enum Phase {
    /// The queue task is not pending.
    Idle,
    /// `Play`'s wait loop; `fresh` while its first pass ran this frame.
    Shown { time: f32, fresh: bool },
    /// The awaited slide-out.
    Hiding,
}

#[derive(Debug, Clone)]
struct Tween {
    tween: FloatTween,
    /// Created inside the tween step: first step on the next frame.
    wait_frame: bool,
    /// The awaited slide-out: its completion continues `Play`.
    awaited: bool,
}

#[derive(Debug, Clone)]
struct Driver {
    start_x: f32,
    queue: VecDeque<usize>,
    phase: Phase,
    x: f32,
    tweens: Vec<Tween>,
}

impl Driver {
    fn playing(&self) -> bool {
        self.phase != Phase::Idle
    }

    /// `cell.Setup(Peek())` and the synchronous start of `Play` up to its
    /// first yield.
    fn start_front(&mut self, dt: f32, in_tween_step: bool, out: &mut Vec<CellCall>) {
        let slot = *self
            .queue
            .front()
            .expect("PlayQueue peeks a non-empty queue");
        out.push(CellCall::Setup { slot });
        out.push(CellCall::Wakeup);
        self.x = self.start_x;
        out.push(CellCall::X(self.x));
        self.tweens.push(Tween {
            tween: FloatTween::new(SHOWN_X, SLIDE_DURATION, Ease::OutCubic),
            wait_frame: in_tween_step,
            awaited: false,
        });
        // The wait loop's first pass: 0 < duration (both durations are
        // positive), so `time += Time.deltaTime` before the first yield. A
        // start before this frame's continuations must not take a second
        // pass in them; a start inside the tween step comes after them.
        self.phase = Phase::Shown {
            time: dt,
            fresh: !in_tween_step,
        };
    }
}

/// The collect-item controller with its five drivers and the view data pool.
#[derive(Debug, Clone)]
pub(crate) struct SlideNotices<T> {
    drivers: Vec<Driver>,
    driver_index: usize,
    pool: Vec<Option<T>>,
    pool_index: usize,
}

/// The duration a pool slot's data carries.
pub(crate) trait NoticeData {
    fn notice_duration(&self) -> f32;
}

impl<T: NoticeData> SlideNotices<T> {
    /// `Initialize`: `anchored_x` is each cell's anchored x after placement.
    pub(crate) fn new(anchored_x: f32) -> Self {
        Self {
            drivers: (0..PARALLEL)
                .map(|_| Driver {
                    start_x: anchored_x + SLIDE_IN_LEFT,
                    queue: VecDeque::new(),
                    phase: Phase::Idle,
                    x: anchored_x,
                    tweens: Vec::new(),
                })
                .collect(),
            driver_index: 0,
            pool: (0..POOL).map(|_| None).collect(),
            pool_index: 0,
        }
    }

    pub(crate) fn slot(&self, slot: usize) -> Option<&T> {
        self.pool.get(slot).and_then(Option::as_ref)
    }

    /// Whether any driver's queue task is pending (`IsPlaying`).
    pub(crate) fn is_playing(&self) -> bool {
        self.drivers.iter().any(Driver::playing)
    }

    /// The queued data count of each driver, the playing one included.
    pub(crate) fn queue_lengths(&self) -> [usize; PARALLEL] {
        std::array::from_fn(|i| self.drivers[i].queue.len())
    }

    /// The layer's `NoticeCollectItem`: the next pool slot takes `data`, and
    /// the controller enqueues it. Returns the slot and the driver it went to;
    /// `calls` gets that driver's cell calls when its queue task starts.
    pub(crate) fn notice(
        &mut self,
        data: T,
        dt: f32,
        calls: &mut Vec<(usize, CellCall)>,
    ) -> (usize, usize) {
        let slot = self.pool_index;
        self.pool[slot] = Some(data);
        self.pool_index = (self.pool_index + 1) % POOL;
        if !self.is_playing() {
            self.driver_index = 0;
        }
        let index = self.driver_index;
        let driver = &mut self.drivers[index];
        driver.queue.push_back(slot);
        if !driver.playing() {
            let mut out = Vec::new();
            driver.start_front(dt, false, &mut out);
            calls.extend(out.into_iter().map(|call| (index, call)));
        }
        self.driver_index = (self.driver_index + 1) % PARALLEL;
        (slot, index)
    }

    /// One frame after the calls: the yield continuations, then the tween
    /// step. `calls` gets every cell call in order.
    pub(crate) fn step(&mut self, dt: f32, calls: &mut Vec<(usize, CellCall)>) {
        for driver in &mut self.drivers {
            let Phase::Shown { time, fresh } = driver.phase.clone() else {
                continue;
            };
            if fresh {
                driver.phase = Phase::Shown { time, fresh: false };
                continue;
            }
            let slot = *driver
                .queue
                .front()
                .expect("a shown driver has its data queued");
            let duration = self.pool[slot]
                .as_ref()
                .expect("a queued slot holds data")
                .notice_duration();
            if time < duration {
                driver.phase = Phase::Shown {
                    time: time + dt,
                    fresh: false,
                };
            } else {
                // `Close` is empty; the awaited slide-out back to the start.
                driver.tweens.push(Tween {
                    tween: FloatTween::new(driver.start_x, SLIDE_DURATION, Ease::OutCubic),
                    wait_frame: false,
                    awaited: true,
                });
                driver.phase = Phase::Hiding;
            }
        }
        for (index, driver) in self.drivers.iter_mut().enumerate() {
            let mut completed = false;
            for tween in &mut driver.tweens {
                if tween.wait_frame {
                    tween.wait_frame = false;
                    continue;
                }
                if let Some(x) = tween.tween.update(driver.x, dt) {
                    driver.x = x;
                    calls.push((index, CellCall::X(x)));
                }
                if tween.awaited && tween.tween.is_complete() {
                    completed = true;
                }
            }
            driver.tweens.retain(|tween| !tween.tween.is_complete());
            if completed && driver.phase == Phase::Hiding {
                calls.push((index, CellCall::Sleep));
                driver.queue.pop_front();
                if driver.queue.is_empty() {
                    driver.phase = Phase::Idle;
                } else {
                    let mut out = Vec::new();
                    driver.start_front(dt, true, &mut out);
                    calls.extend(out.into_iter().map(|call| (index, call)));
                }
            }
        }
    }
}
