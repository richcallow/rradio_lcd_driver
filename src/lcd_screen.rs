pub struct LcdScreen {}

use anyhow::{Context, Result};
use clerk::{DataPins4Lines, Pins};
use core::fmt::Error;
use std::fs::File;
use std::io::prelude::*;
extern crate hex;

mod character_pattern;

enum LCDLineNumbers {
    Line1,
    Line2,
    Line3,
    Line4,
}

impl LCDLineNumbers {
    const NUM_CHARACTERS_PER_LINE: u8 = 20; //the display is visually 20 * 4 characters
    const ROW_OFFSET: u8 = 0x40; //specified by the chip

    fn offset(self) -> u8 {
        match self {
            LCDLineNumbers::Line1 => 0,
            LCDLineNumbers::Line2 => Self::ROW_OFFSET,
            LCDLineNumbers::Line3 => Self::NUM_CHARACTERS_PER_LINE,
            LCDLineNumbers::Line4 => Self::ROW_OFFSET + Self::NUM_CHARACTERS_PER_LINE,
        }
    }
}

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

fn get_line(chip: &mut gpio_cdev::Chip, offset: u32, consumer: &'static str) -> Result<Line> {
    let handle = chip
        .get_line(offset)
        .with_context(|| format!("Failed to get GPIO pin for {:?}", consumer))?
        .request(gpio_cdev::LineRequestFlags::OUTPUT, 0, consumer)
        .with_context(|| format!("GPIO pin for {:?} already in use. Are running another copy of the program elsewhere?", consumer))?;
    Ok(Line { handle })
}
#[derive(Debug, serde::Deserialize)]
struct PinDeclarations {
    rs: u32,     // Register Select
    enable: u32, // Also known as strobe and clock
    data4: u32,
    data5: u32,
    data6: u32,
    data7: u32,
}
impl PinDeclarations {
    fn create_display(
        self,
        chip: &mut gpio_cdev::Chip,
    ) -> Result<
        clerk::Display<
            clerk::ParallelConnection<
                Line,
                FakeLine,
                Line,
                clerk::DataPins4Lines<Line, Line, Line, Line>,
                Delay,
            >,
            clerk::DefaultLines,
        >,
    > {
        let register_select = get_line(chip, self.rs, "register_select")?;
        let read = FakeLine;
        let enable = get_line(chip, self.enable, "enable")?;
        let data4 = get_line(chip, self.data4, "data4")?;
        let data5 = get_line(chip, self.data5, "data5")?;
        let data6 = get_line(chip, self.data6, "data6")?;
        let data7 = get_line(chip, self.data7, "data7")?;

        let pins = Pins {
            register_select,
            read,
            enable,
            data: DataPins4Lines {
                data4,
                data5,
                data6,
                data7,
            },
        };

        let lcd: clerk::Display<
            clerk::ParallelConnection<
                Line,
                FakeLine,
                Line,
                clerk::DataPins4Lines<Line, Line, Line, Line>,
                Delay,
            >,
            clerk::DefaultLines,
        >;

        lcd = clerk::Display::<_, clerk::DefaultLines>::new(pins.into_connection::<Delay>());

        lcd.init(clerk::FunctionSetBuilder::default().set_line_number(clerk::LineNumber::Two)); // screen has 4 lines, but electrically, only 2
        std::thread::sleep(std::time::Duration::from_millis(3)); // with this line commented out, screen goes blank, and cannot be written to subsequently
                                                                 // 1.5 ms is marginal as 1.2ms does not work.

        lcd.set_display_control(
            clerk::DisplayControlBuilder::default() // defaults are display on cursor off blinking off ie cursor is an underscore
                .set_cursor(clerk::CursorState::Off), // normally we want the cursor off
        ); //no extra delay needed here

        lcd.clear();
        std::thread::sleep(std::time::Duration::from_millis(2)); // if this line is commented out, garbage or nothing appears. 1ms is marginal

        Ok(lcd)
    }
}
impl LcdScreen {
    pub fn new() -> Result<Self, anyhow::Error> {
        let pins_src = std::fs::read_to_string("/boot/wiring_pins.toml")
            .context("Failed to read GPIO pin declarations file")?;
        let pins: PinDeclarations =
            toml::from_str(&pins_src).context("Failed to parse GPIO pin declarations file")?;
        println!("GPIO pins {:?}", pins);
        let mut chip = gpio_cdev::Chip::new("/dev/gpiochip0")
            .context("Failed to open GPIO character device")?; // no delay needed here
        let mut lcd = pins.create_display(&mut chip)?;
        lcd.seek_cgram(clerk::SeekFrom::Home(0)); // specify we want to write to the character generator in position 0. Must be a multiple of 8 if we want to start at the start of character

        for character_bitmap in &character_pattern::BITMAPS {
            for row in character_bitmap {
                lcd.write(*row);
            }
        }

        lcd.seek(clerk::SeekFrom::Home(LCDLineNumbers::Line1.offset()));
        for character in "Program starting in new".chars() {
            lcd.write(character as u8);
        }

        println!("Initilised LCD screen");
        Ok(Self {})
    }
    pub fn init() -> clerk::Display<
        clerk::ParallelConnection<
            Line,
            FakeLine,
            Line,
            clerk::DataPins4Lines<Line, Line, Line, Line>,
            Delay,
        >,
        clerk::DefaultLines,
    > {
        let pins_src = std::fs::read_to_string("/boot/wiring_pins.toml")
            .context("Failed to read GPIO pin declarations file")
            .unwrap();

        let pins: PinDeclarations = toml::from_str(&pins_src)
            .context("Failed to parse GPIO pin declarations file")
            .unwrap();
        println!("GPIO pins {:?}", pins);
        let mut chip = gpio_cdev::Chip::new("/dev/gpiochip0")
            .context("Failed to open GPIO character device")
            .unwrap(); // no delay needed here
        let mut lcd = pins.create_display(&mut chip).unwrap();
        lcd.seek_cgram(clerk::SeekFrom::Home(0)); // specify we want to write to the character generator in position 0. Must be a multiple of 8 if we want to start at the start of character

        for character_bitmap in &character_pattern::BITMAPS {
            for row in character_bitmap {
                lcd.write(*row);
            }
        }
        lcd.seek(clerk::SeekFrom::Home(LCDLineNumbers::Line1.offset())); //say all future writes will be characters to be displayed.
        println!("Initialised LCD screen");
        lcd
    }
}
