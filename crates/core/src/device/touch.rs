use std::{sync::Arc, time::Duration};

use base64::{Engine as _, prelude::BASE64_STANDARD};
use opencv::core::Mat;
use tokio_util::sync::CancellationToken;

use super::{
    Adb, Point,
    process::{Invocation, Transport, WriteFrame},
};
use crate::{
    Error, Result,
    calibration::{Resolution, Screen},
    vision::Detector,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TouchDevice {
    node: String,
    slots: usize,
}

impl TouchDevice {
    pub fn node(&self) -> &str {
        &self.node
    }

    pub fn parse(kernel: &str, listing: &str, resolution: Resolution) -> Result<Option<Self>> {
        if !matches!(kernel.trim(), "x86_64" | "aarch64") {
            return Err(Error::Device(
                "Multitouch requires a supported 64-bit Android kernel".into(),
            ));
        }
        for block in listing.split("add device").skip(1) {
            let mut lines = block.lines();
            let Some((_, node)) = lines.next().and_then(|line| line.split_once(':')) else {
                continue;
            };
            let node = node.trim();
            let Some(number) = node.strip_prefix("/dev/input/event") else {
                continue;
            };
            if number.is_empty() || number.len() > 2 || !number.bytes().all(|b| b.is_ascii_digit())
            {
                continue;
            }
            let mut x = None;
            let mut y = None;
            let mut slots = None;
            for line in lines {
                let parse_axis = || -> Option<u32> {
                    let (_, fields) = line.split_once(':')?;
                    let fields: Vec<_> = fields.split(',').map(str::trim).collect();
                    if !fields.contains(&"min 0") {
                        return None;
                    }
                    fields
                        .iter()
                        .find_map(|part| part.strip_prefix("max ")?.parse().ok())
                };
                if line.contains("ABS_MT_POSITION_X") {
                    x = parse_axis();
                }
                if line.contains("ABS_MT_POSITION_Y") {
                    y = parse_axis();
                }
                if line.contains("ABS_MT_SLOT ") {
                    slots = parse_axis();
                }
            }
            if x == resolution.0.checked_sub(1)
                && y == resolution.1.checked_sub(1)
                && let Some(slots) = slots.filter(|n| *n < 32)
            {
                return Ok(Some(Self {
                    node: node.into(),
                    slots: slots as usize + 1,
                }));
            }
        }
        Ok(None)
    }
}

#[derive(Clone, Debug)]
pub enum Gesture {
    Drag {
        from: Point,
        to: Point,
    },
    Pan {
        from: Point,
        to: Point,
    },
    Taps(Vec<Point>),
    TimedTaps {
        points: Vec<Point>,
        spacing: Duration,
    },
    Hold {
        points: Vec<Point>,
        duration: Duration,
    },
    Pinch {
        center: Point,
        start_gap: u32,
        end_gap: u32,
    },
}

impl Gesture {
    fn frames(&self, resolution: Resolution) -> Result<(Vec<WriteFrame>, usize)> {
        let mut frames = Vec::new();
        let fingers = match self {
            Self::Drag { from, to } => {
                from.validate(resolution)?;
                to.validate(resolution)?;
                frames.push(down(&[*from], Duration::from_millis(25)));
                let steps = if from.0.abs_diff(to.0).max(from.1.abs_diff(to.1)) > 160 {
                    24
                } else {
                    12
                };
                for step in 1..=steps {
                    let interpolate = |a: u32, b: u32| {
                        // Slow the final samples so lifting does not fling the army bar.
                        (i64::from(a)
                            + (i64::from(b) - i64::from(a)) * step * (2 * steps - step)
                                / (steps * steps)) as i32
                    };
                    frames.push(WriteFrame {
                        bytes: encode(&[
                            (3, 47, 0),
                            (3, 53, interpolate(from.0, to.0)),
                            (3, 54, interpolate(from.1, to.1)),
                            (0, 0, 0),
                        ]),
                        // Let scrolling settle under the held finger before lifting it.
                        wait: Duration::from_millis(if step == steps { 150 } else { 25 }),
                    });
                }
                frames.push(release(1, Duration::ZERO));
                1
            }
            Self::Pan { from, to } => {
                let pair = |center: Point| -> Result<[Point; 2]> {
                    let points = [
                        Point(center.0.checked_sub(50).ok_or(Error::InputGuard)?, center.1),
                        Point(center.0.checked_add(50).ok_or(Error::InputGuard)?, center.1),
                    ];
                    for point in points {
                        point.validate(resolution)?;
                    }
                    Ok(points)
                };
                pair(*to)?;
                frames.push(down(&pair(*from)?, Duration::from_millis(30)));
                for step in 1..=12 {
                    let interpolate = |a: u32, b: u32| {
                        (i64::from(a) + (i64::from(b) - i64::from(a)) * step * (24 - step) / 144)
                            as u32
                    };
                    let center = Point(interpolate(from.0, to.0), interpolate(from.1, to.1));
                    let mut events = Vec::new();
                    for (slot, point) in pair(center)?.iter().enumerate() {
                        events.extend([
                            (3, 47, slot as i32),
                            (3, 53, point.0 as i32),
                            (3, 54, point.1 as i32),
                        ]);
                    }
                    events.push((0, 0, 0));
                    frames.push(WriteFrame {
                        bytes: encode(&events),
                        // Let the camera settle with both fingers held before releasing.
                        wait: Duration::from_millis(if step == 12 { 150 } else { 25 }),
                    });
                }
                frames.push(release(2, Duration::ZERO));
                2
            }
            Self::Taps(points) => {
                if points.is_empty() || points.len() > 32 {
                    return Err(Error::Device("A tap burst requires 1 to 32 points".into()));
                }
                for &point in points {
                    point.validate(resolution)?;
                    frames.push(down(&[point], Duration::from_millis(30)));
                    frames.push(release(1, Duration::from_millis(30)));
                }
                1
            }
            Self::TimedTaps { points, spacing } => {
                let press = Duration::from_millis(30);
                if points.is_empty()
                    || points.len() > 8
                    || !(Duration::from_millis(100)..=Duration::from_millis(1000)).contains(spacing)
                    || *spacing * (points.len() as u32 - 1) + press > Duration::from_millis(500)
                {
                    return Err(Error::Device(
                        "Timed taps require 1 to 8 points, 100 to 1000 ms spacing and at most 500 ms total".into(),
                    ));
                }
                for (index, &point) in points.iter().enumerate() {
                    point.validate(resolution)?;
                    frames.push(down(&[point], press));
                    frames.push(release(
                        1,
                        if index + 1 == points.len() {
                            Duration::ZERO
                        } else {
                            *spacing - press
                        },
                    ));
                }
                1
            }
            Self::Hold { points, duration } => {
                if points.is_empty()
                    || points.len() > 10
                    || duration.is_zero()
                    || *duration > Duration::from_secs(10)
                {
                    return Err(Error::Device(
                        "A hold requires 1 to 10 fingers for at most 10 seconds".into(),
                    ));
                }
                for point in points {
                    point.validate(resolution)?;
                }
                frames.push(down(points, *duration));
                frames.push(release(points.len(), Duration::ZERO));
                points.len()
            }
            Self::Pinch {
                center,
                start_gap,
                end_gap,
            } => {
                let positions = |gap: u32| -> Result<[Point; 2]> {
                    if gap < 2 {
                        return Err(Error::Device("Pinch fingers must be separated".into()));
                    }
                    let left = center.0.checked_sub(gap / 2).ok_or(Error::InputGuard)?;
                    let right = left.checked_add(gap).ok_or(Error::InputGuard)?;
                    let points = [Point(left, center.1), Point(right, center.1)];
                    for point in points {
                        point.validate(resolution)?;
                    }
                    Ok(points)
                };
                positions(*end_gap)?;
                frames.push(down(&positions(*start_gap)?, Duration::ZERO));
                for step in 1..=10 {
                    let gap = ((i64::from(*start_gap) * 10
                        + (i64::from(*end_gap) - i64::from(*start_gap)) * step)
                        as f64
                        / 10.0)
                        .round_ties_even() as u32;
                    let mut events = Vec::new();
                    for (slot, point) in positions(gap)?.iter().enumerate() {
                        events.extend([(3, 47, slot as i32), (3, 53, point.0 as i32)]);
                    }
                    events.push((0, 0, 0));
                    frames.push(WriteFrame {
                        bytes: encode(&events),
                        wait: Duration::from_millis(20),
                    });
                }
                frames.push(release(2, Duration::ZERO));
                2
            }
        };
        Ok((frames, fingers))
    }
}

fn encode(events: &[(u16, u16, i32)]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(events.len() * 24);
    for (kind, code, value) in events {
        bytes.extend_from_slice(&[0; 16]);
        bytes.extend_from_slice(&kind.to_le_bytes());
        bytes.extend_from_slice(&code.to_le_bytes());
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    bytes
}

fn down(points: &[Point], wait: Duration) -> WriteFrame {
    let mut events = Vec::new();
    for (slot, point) in points.iter().enumerate() {
        events.extend([
            (3, 47, slot as i32),
            (3, 57, slot as i32 + 1),
            (3, 53, point.0 as i32),
            (3, 54, point.1 as i32),
            (3, 58, 1),
        ]);
    }
    events.extend([(1, 330, 1), (0, 0, 0)]);
    WriteFrame {
        bytes: encode(&events),
        wait,
    }
}

fn release(fingers: usize, wait: Duration) -> WriteFrame {
    let mut events = Vec::new();
    for slot in 0..fingers {
        events.extend([(3, 47, slot as i32), (3, 57, -1)]);
    }
    events.extend([(1, 330, 0), (0, 0, 0)]);
    WriteFrame {
        bytes: encode(&events),
        wait,
    }
}

impl<T: Transport> Adb<T> {
    pub async fn touch_device(&self, cancel: &CancellationToken) -> Result<Option<TouchDevice>> {
        let kernel = self
            .command(&["shell", "uname", "-m"], 4096, cancel)
            .await?;
        let events = self
            .command(&["shell", "getevent", "-pl"], 1024 * 1024, cancel)
            .await?;
        TouchDevice::parse(
            &String::from_utf8_lossy(&kernel),
            &String::from_utf8_lossy(&events),
            self.config.resolution,
        )
    }

    pub async fn gesture(
        &mut self,
        detector: Arc<Detector>,
        expected: &[Screen],
        gesture: Gesture,
        cancel: &CancellationToken,
    ) -> Result<()> {
        self.require_live(cancel)?;
        gesture.frames(self.config.resolution)?;
        self.gesture_checked(detector, expected, |_| Ok((gesture, ())), cancel)
            .await
    }

    /// Resolve the complete bounded gesture against the exact frame authorizing input.
    pub async fn gesture_checked<R>(
        &mut self,
        detector: Arc<Detector>,
        expected: &[Screen],
        resolve: impl FnOnce(&Mat) -> Result<(Gesture, R)>,
        cancel: &CancellationToken,
    ) -> Result<R> {
        self.require_live(cancel)?;
        let touch = self
            .touch_device(cancel)
            .await?
            .ok_or_else(|| Error::Device("No matching multitouch device was found".into()))?;
        let frame = self.guard(detector, expected, cancel).await?;
        let (gesture, value) = resolve(&frame.image)?;
        let (frames, fingers) = gesture.frames(self.config.resolution)?;
        if fingers > touch.slots {
            return Err(Error::Device(
                "The touchscreen has too few touch slots".into(),
            ));
        }
        if frame.captured_at.elapsed() > super::MAX_FRAME_AGE {
            return Err(Error::InputGuard);
        }
        self.require_live(cancel)?;
        let replay = concat!(
            "'set -o pipefail || exit; while IFS= read -r frame; do printf %s \"$frame\" | ",
            "toybox base64 -d | toybox dd ibs=4096 obs=24 of=\"$1\" status=none || exit; done'"
        );
        let invocation = |input: Vec<WriteFrame>, timeout| {
            Invocation {
                program: self.config.adb.clone(),
                args: [
                    "-s",
                    &self.config.serial,
                    "shell",
                    "-T",
                    "sh",
                    "-c",
                    replay,
                    "touch",
                    &touch.node,
                ]
                .map(Into::into)
                .to_vec(),
                // Windows adb shell translates binary stdin; send each complete frame as ASCII.
                input: input
                    .into_iter()
                    .map(|frame| WriteFrame {
                        bytes: format!("{}\n", BASE64_STANDARD.encode(frame.bytes)).into_bytes(),
                        wait: frame.wait,
                    })
                    .collect(),
                output_limit: 4096,
                timeout,
            }
        };
        let result = self
            .transport
            .run(invocation(frames, super::COMMAND_TIMEOUT), cancel)
            .await;
        if result.is_err() {
            self.ready = false;
            // Cancellation must still release any fingers already delivered to Android.
            let cleanup = CancellationToken::new();
            if self
                .transport
                .run(
                    invocation(
                        vec![release(fingers, Duration::ZERO)],
                        Duration::from_secs(2),
                    ),
                    &cleanup,
                )
                .await
                .is_err()
            {
                return Err(Error::Device(
                    "Touch release failed; stop the session and check the emulator".into(),
                ));
            }
        }
        result.map_err(super::dispatched_error).map(|_| value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drag_holds_its_exact_endpoint_before_releasing_one_finger() {
        let (frames, fingers) = Gesture::Drag {
            from: Point(640, 650),
            to: Point(608, 638),
        }
        .frames(Resolution(1280, 720))
        .unwrap();
        assert_eq!(fingers, 1);
        assert_eq!(frames.len(), 14);
        assert_eq!(
            frames[0].bytes,
            down(&[Point(640, 650)], Duration::ZERO).bytes
        );
        for (index, (x, y)) in [635, 631, 626, 623, 619, 616, 614, 612, 610, 609, 609, 608]
            .into_iter()
            .zip([649, 647, 645, 644, 643, 641, 641, 640, 639, 639, 639, 638])
            .enumerate()
        {
            assert_eq!(
                frames[index + 1].bytes,
                encode(&[(3, 47, 0), (3, 53, x), (3, 54, y), (0, 0, 0),])
            );
        }
        assert!(
            frames[..12]
                .iter()
                .all(|frame| frame.wait == Duration::from_millis(25))
        );
        assert_eq!(frames[12].wait, Duration::from_millis(150));
        assert_eq!(frames[13].bytes, release(1, Duration::ZERO).bytes);
        assert_eq!(frames[13].wait, Duration::ZERO);
        assert_eq!(
            frames.iter().map(|frame| frame.wait).sum::<Duration>(),
            Duration::from_millis(450)
        );
        assert!(
            Gesture::Drag {
                from: Point(1280, 0),
                to: Point(608, 638)
            }
            .frames(Resolution(1280, 720))
            .is_err()
        );
        assert!(
            Gesture::Drag {
                from: Point(640, 650),
                to: Point(0, 720)
            }
            .frames(Resolution(1280, 720))
            .is_err()
        );
    }

    #[test]
    fn long_drags_slow_down_without_extending_their_endpoint() {
        let (frames, fingers) = Gesture::Drag {
            from: Point(820, 651),
            to: Point(540, 651),
        }
        .frames(Resolution(1280, 720))
        .unwrap();
        assert_eq!(fingers, 1);
        assert_eq!(frames.len(), 26);
        assert_eq!(
            frames.iter().map(|frame| frame.wait).sum::<Duration>(),
            Duration::from_millis(750)
        );
        assert_eq!(
            frames[24].bytes,
            encode(&[(3, 47, 0), (3, 53, 540), (3, 54, 651), (0, 0, 0)])
        );
        assert_eq!(frames[24].wait, Duration::from_millis(150));
        assert_eq!(frames[25].bytes, release(1, Duration::ZERO).bytes);
    }

    #[test]
    fn timed_taps_preserve_spacing_release_and_burst_bounds() {
        let (frames, fingers) = Gesture::TimedTaps {
            points: vec![Point(10, 20); 5],
            spacing: Duration::from_millis(100),
        }
        .frames(Resolution(1280, 720))
        .unwrap();
        assert_eq!(fingers, 1);
        assert_eq!(frames.len(), 10);
        assert_eq!(
            frames.iter().map(|frame| frame.wait).sum::<Duration>(),
            Duration::from_millis(430)
        );
        for (index, pair) in frames.chunks_exact(2).enumerate() {
            assert_eq!(pair[0].wait, Duration::from_millis(30));
            assert_eq!(
                pair[1].wait,
                if index == 4 {
                    Duration::ZERO
                } else {
                    Duration::from_millis(70)
                }
            );
            assert_eq!(pair[1].bytes, release(1, Duration::ZERO).bytes);
        }
        for (count, spacing) in [(0, 100), (9, 100), (6, 100), (2, 500), (1, 99), (1, 1001)] {
            assert!(
                Gesture::TimedTaps {
                    points: vec![Point(10, 20); count],
                    spacing: Duration::from_millis(spacing),
                }
                .frames(Resolution(1280, 720))
                .is_err()
            );
        }
        assert!(
            Gesture::TimedTaps {
                points: vec![Point(1280, 0)],
                spacing: Duration::from_millis(100),
            }
            .frames(Resolution(1280, 720))
            .is_err()
        );
        let (frames, _) = Gesture::TimedTaps {
            points: vec![Point(10, 20)],
            spacing: Duration::from_millis(1000),
        }
        .frames(Resolution(1280, 720))
        .unwrap();
        assert_eq!(
            frames.iter().map(|frame| frame.wait).sum::<Duration>(),
            Duration::from_millis(30)
        );
    }

    #[test]
    fn camera_pan_holds_both_fingers_at_the_endpoint_before_releasing() {
        let (frames, fingers) = Gesture::Pan {
            from: Point(640, 320),
            to: Point(440, 160),
        }
        .frames(Resolution(1280, 720))
        .unwrap();
        assert_eq!(fingers, 2);
        assert_eq!(frames.len(), 14);
        let events = |frame: &WriteFrame| {
            frame
                .bytes
                .chunks_exact(24)
                .map(|b| {
                    (
                        u16::from_le_bytes(b[16..18].try_into().unwrap()),
                        u16::from_le_bytes(b[18..20].try_into().unwrap()),
                        i32::from_le_bytes(b[20..24].try_into().unwrap()),
                    )
                })
                .collect::<Vec<_>>()
        };
        assert!(events(&frames[0]).contains(&(3, 57, 2)));
        assert_eq!(frames[0].wait, Duration::from_millis(30));
        for frame in &frames[1..12] {
            assert_eq!(frame.wait, Duration::from_millis(25));
            assert!(!events(frame).contains(&(3, 57, -1)));
        }
        let final_move = events(&frames[12]);
        assert!(final_move.contains(&(3, 53, 390)) && final_move.contains(&(3, 53, 490)));
        assert_eq!(final_move.iter().filter(|e| **e == (3, 54, 160)).count(), 2);
        assert!(!final_move.contains(&(3, 57, -1)));
        assert_eq!(frames[12].wait, Duration::from_millis(150));
        assert_eq!(frames[13].wait, Duration::ZERO);
        assert_eq!(
            events(&frames[13])
                .iter()
                .filter(|e| **e == (3, 57, -1))
                .count(),
            2
        );
        assert!(
            Gesture::Pan {
                from: Point(40, 320),
                to: Point(640, 320)
            }
            .frames(Resolution(1280, 720))
            .is_err()
        );
    }

    #[test]
    fn events_use_64_bit_headers_and_release_every_finger() {
        let frames = Gesture::Hold {
            points: vec![Point(1, 2), Point(3, 4)],
            duration: Duration::from_secs(1),
        }
        .frames(Resolution(1280, 720))
        .unwrap()
        .0;
        assert_eq!(frames[0].bytes.len(), 12 * 24);
        assert_eq!(&frames[0].bytes[16..24], &[3, 0, 47, 0, 0, 0, 0, 0]);
        assert_eq!(
            &frames[0].bytes[2 * 24 + 16..3 * 24],
            &[3, 0, 53, 0, 1, 0, 0, 0]
        );
        assert_eq!(
            frames[1]
                .bytes
                .chunks_exact(24)
                .filter(|event| event[16..20] == [3, 0, 57, 0] && event[20..24] == [255; 4])
                .count(),
            2
        );
        assert!(
            Gesture::Hold {
                points: vec![Point(1, 2)],
                duration: Duration::MAX
            }
            .frames(Resolution(1280, 720))
            .is_err()
        );
        assert!(
            Gesture::Taps(vec![Point(1280, 0)])
                .frames(Resolution(1280, 720))
                .is_err()
        );
        assert!(
            Gesture::Pinch {
                center: Point(0, 0),
                start_gap: 200,
                end_gap: 20
            }
            .frames(Resolution(1280, 720))
            .is_err()
        );
    }
}
