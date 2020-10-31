pub struct FakeLine;

impl clerk::DisplayHardwareLayer for FakeLine {
    fn set_level(&self, _level: clerk::Level) {}
    fn set_direction(&self, _direction: clerk::Direction) {}
    fn get_value(&self) -> u8 {
        0
    }
}

pub struct Line {
    handle: gpio_cdev::LineHandle,
}

impl Line {
    pub fn new(handle: gpio_cdev::LineHandle) -> Self {
        Self { handle }
    }
}

impl clerk::DisplayHardwareLayer for Line {
    fn set_level(&self, level: clerk::Level) {
        self.handle
            .set_value(match level {
                clerk::Level::Low => 0,
                clerk::Level::High => 1,
            })
            .unwrap();
    }
    fn set_direction(&self, _direction: clerk::Direction) {}

    fn get_value(&self) -> u8 {
        0
    }
}

pub struct Delay;

impl clerk::Delay for Delay {
    const ADDRESS_SETUP_TIME: u16 = 60;
    const ENABLE_PULSE_WIDTH: u16 = 300; // 300ns in the spec sheet 450;
    const DATA_HOLD_TIME: u16 = 10; // 10ns in the spec sheet  20;
    const COMMAND_EXECUTION_TIME: u16 = 37;

    fn delay_ns(ns: u16) {
        std::thread::sleep(std::time::Duration::from_nanos(u64::from(ns)));
    }
}
