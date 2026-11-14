//! Windows.Gaming.Input's output for an Xbox One pad: the only Windows API that drives the
//! impulse-trigger motors of a pad on a cable, and one that shares the pad with other programs
//! instead of taking its HID endpoint. Windows lists an Xbox pad as a WGI gamepad and as a raw
//! HID entry; only the first is a WGI gamepad, which is the one this opens.
use crate::xbox::{Motors, is_xbox_one};
use windows::Gaming::Input::{Gamepad, GamepadVibration, RawGameController};

/// A pad WGI can make vibrate.
pub struct Pad {
    gamepad: Gamepad,
    wireless: bool,
}

impl Pad {
    /// The Xbox One pad WGI lists that is also a WGI gamepad (the raw HID entry beside it is not):
    /// a wired one before a wireless one, because a pad on a cable can still be listed over
    /// Bluetooth for a moment. `None` when there is none; an error when WGI itself is unavailable.
    pub fn open() -> windows::core::Result<Option<Pad>> {
        let controllers = RawGameController::RawGameControllers()?;
        // Not `into_iter()`: it crashes when the game runs through Steam (gilrs-core #132).
        let count = controllers.Size()?;
        let mut wireless = None;
        for index in 0..count {
            let Ok(controller) = controllers.GetAt(index) else {
                continue;
            };
            let (Ok(vendor), Ok(product)) = (
                controller.HardwareVendorId(),
                controller.HardwareProductId(),
            ) else {
                continue;
            };
            if !is_xbox_one(vendor, product) {
                continue;
            }
            let Ok(gamepad) = Gamepad::FromGameController(&controller) else {
                continue;
            };
            let is_wireless = controller.IsWireless().unwrap_or(false);
            let pad = Pad {
                gamepad,
                wireless: is_wireless,
            };
            if !is_wireless {
                return Ok(Some(pad));
            }
            wireless = wireless.or(Some(pad));
        }
        Ok(wireless)
    }

    /// Whether the system says the pad is on a wireless link: a wired pad goes through WGI's
    /// four motors even when Windows still lists a Bluetooth entry for it.
    pub fn wireless(&self) -> bool {
        self.wireless
    }

    /// All four motors, 0..1. The handles alone also work through gilrs' own WGI force feedback;
    /// the trigger motors are the reason this exists.
    pub fn write(&mut self, motors: Motors) -> windows::core::Result<()> {
        self.gamepad.SetVibration(vibration(motors))
    }
}

impl Drop for Pad {
    fn drop(&mut self) {
        // The pad keeps the last levels while this object lives; quitting must not leave its
        // motors running.
        let _ = self.gamepad.SetVibration(vibration(Motors::STOP));
    }
}

/// The four levels WGI takes, 0..1 each: the heavy and light handles, then the triggers.
fn vibration(motors: Motors) -> GamepadVibration {
    let level = |level: u8| f64::from(level) / 255.0;
    GamepadVibration {
        LeftMotor: level(motors.strong),
        RightMotor: level(motors.weak),
        LeftTrigger: level(motors.left_trigger),
        RightTrigger: level(motors.right_trigger),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_motor_keeps_its_own_channel() {
        let all = vibration(Motors {
            strong: 255,
            weak: 0,
            left_trigger: 0,
            right_trigger: 255,
        });
        assert_eq!(all.LeftMotor, 1.0);
        assert_eq!(all.RightMotor, 0.0);
        assert_eq!(all.LeftTrigger, 0.0);
        assert_eq!(all.RightTrigger, 1.0);
        let stopped = vibration(Motors::STOP);
        assert_eq!(
            (
                stopped.LeftMotor,
                stopped.RightMotor,
                stopped.LeftTrigger,
                stopped.RightTrigger
            ),
            (0.0, 0.0, 0.0, 0.0)
        );
    }
}
