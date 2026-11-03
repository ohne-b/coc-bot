use std::{sync::Arc, time::Duration};

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
    Taps(Vec<Point>),
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
        let (frames, fingers) = gesture.frames(self.config.resolution)?;
        let touch = self
            .touch_device(cancel)
            .await?
            .ok_or_else(|| Error::Device("No matching multitouch device was found".into()))?;
        if fingers > touch.slots {
            return Err(Error::Device(
                "The touchscreen has too few touch slots".into(),
            ));
        }
        self.guard(detector, expected, cancel).await?;
        let invocation = |input, timeout| Invocation {
            program: self.config.adb.clone(),
            args: [
                "-s",
                &self.config.serial,
                "exec-in",
                "dd",
                &format!("of={}", touch.node),
                // Separate sizes buffer short pipe reads into complete input_event records.
                "ibs=24",
                "obs=24",
            ]
            .map(Into::into)
            .to_vec(),
            input,
            output_limit: 4096,
            timeout,
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
        result.map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
