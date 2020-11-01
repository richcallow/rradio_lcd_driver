use anyhow::Context;

mod character_pattern;
mod hal;

#[derive(Clone, Copy)]
pub enum LCDLineNumbers {
    Line1,
    Line2,
    Line3,
    Line4,
}

impl LCDLineNumbers {
    pub const NUM_CHARACTERS_PER_LINE: u8 = 20; //the display is visually 20 * 4 characters
    pub const ROW_OFFSET: u8 = 0x40; //specified by the chip

    fn offset(self) -> u8 {
        match self {
            LCDLineNumbers::Line1 => 0,
            LCDLineNumbers::Line2 => Self::ROW_OFFSET,
            LCDLineNumbers::Line3 => Self::NUM_CHARACTERS_PER_LINE,
            LCDLineNumbers::Line4 => Self::ROW_OFFSET + Self::NUM_CHARACTERS_PER_LINE,
        }
    }
}

pub type ClerkDisplay = clerk::Display<
    clerk::ParallelConnection<
        hal::Line,
        hal::FakeLine,
        hal::Line,
        clerk::DataPins4Lines<hal::Line, hal::Line, hal::Line, hal::Line>,
        hal::Delay,
    >,
    clerk::DefaultLines,
>;

pub struct LcdScreen {
    lcd: ClerkDisplay,
}

impl LcdScreen {
    pub fn new() -> anyhow::Result<Self> {
        let pins_src = std::fs::read_to_string("/boot/wiring_pins.toml")
            .context("Failed to read GPIO pin declarations file")?;

        let pins: PinDeclarations =
            toml::from_str(&pins_src).context("Failed to parse GPIO pin declarations file")?;
        println!("GPIO pins {:?}", pins);
        let mut chip = gpio_cdev::Chip::new("/dev/gpiochip0")
            .context("Failed to open GPIO character device")?; // no delay needed here
        let mut lcd = pins
            .create_display(&mut chip)
            .context("Could not create display")?;
        lcd.clear();
        std::thread::sleep(std::time::Duration::from_millis(3));
        lcd.seek_cgram(clerk::SeekFrom::Home(0)); // specify we want to write to the character generator in position 0. Must be a multiple of 8 if we want to start at the start of character
        for character_bitmap in &character_pattern::BITMAPS {
            for row in character_bitmap {
                lcd.write(*row);
            }
        }
        lcd.seek(clerk::SeekFrom::Home(LCDLineNumbers::Line1.offset())); //say all future writes will be characters to be displayed.
        println!("Initialised LCD screen");
        Ok(Self { lcd })
    }

    pub fn clear(&self) {
        self.lcd.clear();
        std::thread::sleep(std::time::Duration::from_millis(3));
    }

    pub fn write_buffer_state(&mut self, buffer_position: u8) {
        self.lcd
            .seek(clerk::SeekFrom::Home(LCDLineNumbers::Line4.offset()));
        let trimmed_buffer = buffer_position.min(99); //0 to 100 is 101 values, & the screen only handles 100 values, so trim downwards
        #[allow(clippy::cast_possible_wrap)]
        let scaled_buffer = (trimmed_buffer / 5) as i8; //the characters have 5 columns
        for _count in 0..scaled_buffer {
            self.lcd.write(' ' as u8); //first write space in all the character positions before the cursor
        }
        self.lcd.write((trimmed_buffer % 5) as u8); //then write the apppriate cursor character in the next position
        for _count in scaled_buffer + 1..20 {
            self.lcd.write(' ' as u8); //then clear the rest of the line
        }
    }

    pub fn write_ascii(&mut self, line: LCDLineNumbers, position: u8, string: String) {
        self.lcd
            .seek(clerk::SeekFrom::Home(line.offset() + position));
        for character in string.chars() {
            self.lcd.write(character as u8);
        }
    }
    pub fn write_utf8(&mut self, line: LCDLineNumbers, position: u8, string: String) {
        self.lcd
            .seek(clerk::SeekFrom::Home(line.offset() + position));
        for unicode_character in string.chars() {
            if unicode_character < '~' {
                // characters lower than ~ are handled by the built-in character set
                self.lcd.write(unicode_character as u8)
            } else {
                let ascii_character_bytes = match unicode_character {
                    'é' => &[5], // e accute fifth bespoke character defined starting with the zeroeth bespoke character
                    'è' => &[6], // e grave
                    'à' => &[7], // a grave
                    'ä' => &[0xE1], // a umlaut            // see look up table in GDM2004D.pdf page 9/9
                    'ñ' => &[0xEE], // n tilde
                    'ö' => &[0xEF], // o umlaut
                    'ü' => &[0xF5], // u umlaut
                    'π' => &[0xE4], // pi
                    'µ' => &[0xF7], // mu
                    '~' => &[0xF3], // cannot display tilde using the standard character set in GDM2004D.pdf. This is the best we can do.
                    '' => &[0xFF], // <Control>  = 0x80 replaced by splodge
                    _ => unidecode::unidecode_char(unicode_character).as_bytes(),
                };
                for octet in ascii_character_bytes {
                    self.lcd.write(*octet);
                }
            }
        }
    }
    pub fn write_line(&mut self, line: LCDLineNumbers, length: usize, string: String) {
        self.lcd.seek(clerk::SeekFrom::Home(line.offset()));
        let string_to_output = format!(
            "{Thestring:<Width$.Width$}",
            Thestring = string,
            Width = length
        );
        for unicode_character in string_to_output.chars() {
            if unicode_character < '~' {
                // characters lower than ~ are handled by the built-in character set
                self.lcd.write(unicode_character as u8)
            } else {
                let ascii_character_bytes = match unicode_character {
                    'é' => &[5], // e accute fifth bespoke character defined starting with the zeroeth bespoke character
                    'è' => &[6], // e grave
                    'à' => &[7], // a grave
                    'ä' => &[0xE1], // a umlaut            // see look up table in GDM2004D.pdf page 9/9
                    'ñ' => &[0xEE], // n tilde
                    'ö' => &[0xEF], // o umlaut
                    'ü' => &[0xF5], // u umlaut
                    'π' => &[0xE4], // pi
                    'µ' => &[0xF7], // mu
                    '~' => &[0xF3], // cannot display tilde using the standard character set in GDM2004D.pdf. This is the best we can do.
                    '' => &[0xFF], // <Control>  = 0x80 replaced by splodge
                    _ => unidecode::unidecode_char(unicode_character).as_bytes(),
                };
                for octet in ascii_character_bytes {
                    self.lcd.write(*octet);
                }
            }
        }
    }
    pub fn write_multiline(&mut self, line: LCDLineNumbers, length: usize, string: String) {
        let mut local_line = &line;
        self.lcd.seek(clerk::SeekFrom::Home(local_line.offset()));
        let string_to_output = format!(
            "{Thestring:<Width$.Width$}",
            Thestring = string,
            Width = length
        );
        let mut num_characters_written: u8 = 0;
        for unicode_character in string_to_output.chars() {
            if unicode_character < '~' {
                // characters lower than ~ are handled by the built-in character set
                self.lcd.write(unicode_character as u8);
                num_characters_written += 1;
                if num_characters_written >= LCDLineNumbers::NUM_CHARACTERS_PER_LINE {
                    num_characters_written = 0;
                    match local_line {
                        LCDLineNumbers::Line1 => local_line = &LCDLineNumbers::Line2,
                        LCDLineNumbers::Line2 => local_line = &LCDLineNumbers::Line3,
                        LCDLineNumbers::Line3 => local_line = &LCDLineNumbers::Line4,
                        LCDLineNumbers::Line4 => local_line = &LCDLineNumbers::Line1,
                    }
                    self.lcd.seek(clerk::SeekFrom::Home(local_line.offset()));
                }
            } else {
                let ascii_character_bytes = match unicode_character {
                    'é' => &[5], // e accute fifth bespoke character defined starting with the zeroeth bespoke character
                    'è' => &[6], // e grave
                    'à' => &[7], // a grave
                    'ä' => &[0xE1], // a umlaut            // see look up table in GDM2004D.pdf page 9/9
                    'ñ' => &[0xEE], // n tilde
                    'ö' => &[0xEF], // o umlaut
                    'ü' => &[0xF5], // u umlaut
                    'π' => &[0xE4], // pi
                    'µ' => &[0xF7], // mu
                    '~' => &[0xF3], // cannot display tilde using the standard character set in GDM2004D.pdf. This is the best we can do.
                    '' => &[0xFF], // <Control>  = 0x80 replaced by splodge
                    _ => unidecode::unidecode_char(unicode_character).as_bytes(),
                };
                for octet in ascii_character_bytes {
                    self.lcd.write(*octet);
                    num_characters_written += 1;
                    if num_characters_written >= LCDLineNumbers::NUM_CHARACTERS_PER_LINE {
                        num_characters_written = 0;
                        match local_line {
                            LCDLineNumbers::Line1 => local_line = &LCDLineNumbers::Line2,
                            LCDLineNumbers::Line2 => local_line = &LCDLineNumbers::Line3,
                            LCDLineNumbers::Line3 => local_line = &LCDLineNumbers::Line4,
                            LCDLineNumbers::Line4 => local_line = &LCDLineNumbers::Line1,
                        }
                        self.lcd.seek(clerk::SeekFrom::Home(local_line.offset()));
                    }
                }
            }
        }
    }
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
    fn create_display(self, chip: &mut gpio_cdev::Chip) -> Result<ClerkDisplay, anyhow::Error> {
        let register_select = get_line(chip, self.rs, "register_select")?;
        let read = hal::FakeLine;
        let enable = get_line(chip, self.enable, "enable")?;
        let data4 = get_line(chip, self.data4, "data4")?;
        let data5 = get_line(chip, self.data5, "data5")?;
        let data6 = get_line(chip, self.data6, "data6")?;
        let data7 = get_line(chip, self.data7, "data7")?;

        let pins = clerk::Pins {
            register_select,
            read,
            enable,
            data: clerk::DataPins4Lines {
                data4,
                data5,
                data6,
                data7,
            },
        };

        let lcd =
            clerk::Display::<_, clerk::DefaultLines>::new(pins.into_connection::<hal::Delay>());

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

fn get_line(
    chip: &mut gpio_cdev::Chip,
    offset: u32,
    consumer: &'static str,
) -> Result<hal::Line, anyhow::Error> {
    let handle = chip
        .get_line(offset)
        .with_context(|| format!("Failed to get GPIO pin for {:?}", consumer))?
        .request(gpio_cdev::LineRequestFlags::OUTPUT, 0, consumer)
        .with_context(|| format!("GPIO pin for {:?} already in use. Are running another copy of the program elsewhere?", consumer))?;
    Ok(hal::Line::new(handle))
}
