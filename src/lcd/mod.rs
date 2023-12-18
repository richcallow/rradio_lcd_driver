/*
The lines below instruct the Pi to use the hd44780 driver & load it
(or equivalent if different pins are to be used.) The pin numbers specified are GPIO pin numbers
    dtoverlay=hd44780-lcd,pin_rs=16,pin_en=12,display_height=4,display_width=20
    dtparam=pin_d4=24,pin_d5=23,pin_d6=25,pin_d7=9
*/
use anyhow::Context;
use chrono::Local;
use rradio_messages::PipelineState;
use std::io::Write;

mod character_pattern;
mod get_temperature;
mod get_wifi_strength;

#[derive(PartialEq, Debug)]
pub enum LineNum {
    Line1,
    Line2,
    Line3,
    Line4,
}

pub const NUM_CHARACTERS_PER_LINE: usize = 20; //the display is visually 20 * 4 characters

pub const VOLUME_CHAR_COUNT: usize = 7;
pub const LINE1_DATA_CHAR_COUNT: u16 = (NUM_CHARACTERS_PER_LINE - VOLUME_CHAR_COUNT) as u16;
pub const LINE1_DATA_CHAR_COUNT_USIZE: usize = LINE1_DATA_CHAR_COUNT as usize;

impl LineNum {
    fn into_u16(self) -> u16 {
        match self {
            LineNum::Line1 => 0,
            LineNum::Line2 => 1,
            LineNum::Line3 => 2,
            LineNum::Line4 => 3,
        }
    }
}

pub struct Lc {
    lcd_file: std::fs::File,
}

impl Lc {
    fn clear_screen(mut lcd_file: impl std::io::Write) {
        if let Err(err) = write!(lcd_file, "\x1b[LI\x1b[Lb\x1b[Lc") {
            // initialises the screen & stops the cursor blinking & turns the cursor off
            println!("Failed to initialise the screen : {err}");
        }

        // generate the cursors in positions 0 to 7 of the character generator, as the initialisation MIGHt have cleared it
        for char_count in 0..8 {
            let mut out_string = format!("\x1b[LG{:01x}", char_count);
            for col_count in 0..8 {
                let s = format!("{:02x}", character_pattern::BITMAPS[char_count][col_count]);
                out_string = out_string + &s;
            }
            out_string.push(';');

            if let Err(err) = write!(lcd_file, "{}", out_string) {
                println!("Failed to initialise the screen : {err}");
            }

            /*
            the first five strings that software generates & sends are
            const INIT_STRING0: &str = "\x1b[LG0101010101010101f;";
            const INIT_STRING1: &str = "\x1b[LG1080808080808081f;";
            const INIT_STRING2: &str = "\x1b[LG2040404040404041f;";
            const INIT_STRING3: &str = "\x1b[LG3020202020202021f;";
            const INIT_STRING4: &str = "\x1b[LG4010101010101011f;";

            write!(lcd_file, "\x1b[LI\x1b[Lb\x1b[LC") // initialise the screen & stop the cursor blinking & turn the cursor on
                .context("Failed to initialise the screen")?;

            write!(lcd_file, "{}", INIT_STRING0) // write the cursor symbol
                .context("Failed to initialise the screen")?;

            write!(lcd_file, "{}", INIT_STRING1) // write the cursor symbol
                .context("Failed to initialise the screen")?;
            write!(lcd_file, "{}", INIT_STRING2) // write the cursor symbol
                .context("Failed to initialise the screen")?;
            write!(lcd_file, "{}", INIT_STRING3) // write the cursor symbol
                .context("Failed to initialise the screen")?;
            write!(lcd_file, "{}", INIT_STRING4) // write the cursor symbol
                .context("Failed to initialise the screen")?;
            */

            /*println!(
                "initialised character {} with string {}",
                char_count, out_string
            );*/
        }
    }

    pub fn new() -> anyhow::Result<Self> {
        let lcd_file = std::fs::File::options()
            .write(true)
            .open("/dev/lcd")
            .context("Failed to open LCD file even in main")?;

        Self::clear_screen(&lcd_file);
        //println!("Initialised the LCD screen");

        Ok(Lc { lcd_file })
    }

    pub fn clear(&mut self) {
        Self::clear_screen(&mut self.lcd_file);
    }

    /// write_ascii writes the specified string to the line & column specified. It is assumed that the characters are ASCII.
    /// The characters must not be too long to fit on the specified line in the specified position
    pub fn write_ascii(&mut self, line_number: LineNum, column: u16, input: &str) {
        let line_number = line_number.into_u16();
        if let Err(err) = write!(self.lcd_file, "\x1b[Lx{column}y{line_number};{}", input) {
            println!("in write_ascii, Failed to write to LCD screen : {err}");
        }
    }
    /// write_multiline writes exactly the specified number of characters, which can be less than one line
    /// It any character is not ASCII, it is transliterated.
    pub fn write_multiline(&mut self, line_number: LineNum, length: usize, in_string: &str) {
        let line_number = line_number.into_u16(); // convert the line number from an enum to u16
        if let Err(err) = write!(self.lcd_file, "\x1b[Lx0y{line_number};") {
            // move the cursor to the start of the specified line
            println!("in write_multiline, Failed to write move the cursor : {err}");
        }

        let mut output_string = Vec::new();
        for one_char in in_string.chars() {
            if one_char < '~' {
                output_string.push(one_char as u8);
            } else {
                output_string.extend_from_slice(match one_char {
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
                    _ => unidecode::unidecode_char(one_char).as_bytes(),
                });
            }
        }

        output_string.resize(length, b' ');

        let mut lines = output_string.chunks_exact(NUM_CHARACTERS_PER_LINE);
        for line in lines.by_ref() {
            self.lcd_file.write_all(line).expect("Failed in write_all");
            self.lcd_file
                .write_all(b"\n")
                .expect("Failed in new line in write_all");
        }
        self.lcd_file
            .write_all(lines.remainder())
            .expect("Failed to write rest in write_all");
    }

    /// write_volume outputs the volume (or the gstreamer state if not playing, or "  Muted") to the LCD screen
    pub fn write_volume(&mut self, pipe_line_state: PipelineState, is_muted: bool, volume: i32) {
        //println!("in function write_volume: state {}", pipe_line_state);

        let message = if (pipe_line_state == PipelineState::Playing) && volume >= 0 {
            if is_muted {
                "  Muted".to_string() // 2 spaces in order to right justify
            } else {
                format!(
                    "Vol{:>Width$.Width$}",
                    volume,
                    Width = VOLUME_CHAR_COUNT - 3
                )
            }
        } else {
            format!(
                "{:<Width$.Width$}",
                pipe_line_state.to_string(),
                Width = VOLUME_CHAR_COUNT
            ) // if we use pipeline_state.to_string() without the .to_string, the result can be less than 7 characters long
        };
        self.write_ascii(LineNum::Line1, LINE1_DATA_CHAR_COUNT, message.as_str())
    }

    /// get_cpu_temperature gets the CPU temperature as an integer
    pub fn get_cpu_temperature(&mut self) -> i32 {
        get_temperature::get_cpu_temperature()
    }
    /// write_temperature_and_strength writes the CPU temperature & the Wi-Fi signal strength to the specified line
    pub fn write_temperature_and_strength(&mut self, line_number: LineNum) {
        self.write_multiline(
            line_number,
            NUM_CHARACTERS_PER_LINE,
            &format!(
                "CPU Temp {}C WiFi{}",
                get_temperature::get_cpu_temperature(),
                get_wifi_strength::get_wifi_signal_strength()
            ),
        )
    }

    ///   write_date_and_time_of_day_line3 writes the date & time to line 3 of the screen
    pub fn write_date_and_time_of_day_line3(&mut self) {
        // writes the time of day to line 3
        self.write_multiline(
            LineNum::Line3,
            NUM_CHARACTERS_PER_LINE,
            Local::now()
                .format("  %d %b %y %H:%M:%SS")
                .to_string()
                .as_str(),
        )
    }

    /// write_temperature_and_time_to_line4 writes the temperature & time to line 4
    pub fn write_temperature_and_time_to_line4(&mut self) {
        self.write_multiline(
            LineNum::Line4,
            NUM_CHARACTERS_PER_LINE,
            &format!(
                "CPU temp {} C {}",
                get_temperature::get_cpu_temperature(),
                Local::now().format("%H:%M").to_string().as_str(),
            ),
        )
    }

    /// write_all_line_2 takes the specifed string & shortens it or lengthens it to exactly fill line 2
    pub fn write_all_line_2(&mut self, string: &str) {
        self.write_multiline(LineNum::Line2, NUM_CHARACTERS_PER_LINE, string);
    }

    /// write_buffer_state writes a cursor to line 4 showing how full the gsteamer buffer is
    pub fn write_buffer_state(&mut self, buffer_position: u8) {
        // writes the state of the gstreamer buffer on the 4th line as a moving cursor
        if let Err(err) = write!(self.lcd_file, "\x1b[Lx0y3;") {
            // move the cursor to the start of the specified line
            println!("in write_buffer_state, Failed to write move the cursor : {err}");
        }
        let trimmed_buffer = buffer_position.min(99); // 0 to 100 is 101 values, & the screen only handles 100 values, so trim downwards
        #[allow(clippy::cast_possible_wrap)]
        let scaled_buffer = (trimmed_buffer / 5) as i8; // the characters have 5 columns
        for _count in 0..scaled_buffer {
            if let Err(err) = write!(self.lcd_file, " ") {
                // first write space in all the character positions before the cursor
                println!("in write_buffer_state, Failed to write space before the cursor : {err}");
            }
        }
        self.lcd_file
            .write_all(&[(trimmed_buffer % 5)])
            .expect("in write_buffer_state, Failed to write the cursor");

        for _count in scaled_buffer + 1..20 {
            if let Err(err) = write!(self.lcd_file, " ") {
                // then clear the rest of the line
                println!("in write_buffer_state, Failed to write space after the cursor : {err}");
            }
        }
    }

    /// write_with_scroll writes with a scroll, as appropriate the string with the wanted scroll value to the screen
    ///
    pub fn write_with_scroll(
        &mut self,
        line: LineNum,        // the line to write to
        length: usize, // length of the screen  to be used; typically a multiple of the line length
        string: &str,  // The string to write
        position: &mut usize, // the scroll position; it is mutable so that sucessive calls scroll the line.
    ) {
        if string.is_char_boundary(*position) {
            // check that the preceding decrements of position have solved the char boundary problem
            self.write_multiline(line, length, &string[*position..]); // as they have, we can safely call this line
        }
        let mut found_space = false;
        for i in *position + 6..*position + 14 {
            // scroll to the next space, if it is reasonably soon, but not too soon
            if i + 1 < string.len() {
                // note "string.len() -1) is largw and positive if the string is null
                // check the index stays within bounds
                if string.as_bytes()[i] == b' ' {
                    *position = i;
                    found_space = true;
                    break;
                }
            } else {
                println!(
                    "Breaking to avoid going beyond end of string, which is \"{}\"",
                    string
                );
                break;
            }
        }
        if !found_space {
            *position += 6;
        }

        *position += 1; // Advance past the space (if the "for" loop found one), else advance 1 byte (perhaps not 1 character!)
        if *position > string.len() - 10 {
            // If we are almost at the end of the start again. Also ensure that position remains in bounds
            *position = 0;
        }
        if (*position > 0) && !(&string.is_char_boundary(*position)) {
            *position -= 1 // if we are not on a character boundary because we have a multi-byte character bring the pointer back one byte to be on a boundary
        }
        if (*position > 0) && !(&string.is_char_boundary(*position)) {
            *position -= 1 // second attempt; a few characters use 3 bytes
        }
    }
}

/*
\f" will clear the display and put the cursor home.

"\x1b[LD" will enable the display, "\x1b[Ld" will disable it.
"\x1b[LC" will turn the cursor on, "\x1b[Lc" will turn it off.
"\x1b[LB" will enable blink. "\x1b[Lb" will disable it.
"\x1b[LL" will shift the display left. "\x1b[LR" will shift it right.
"\x1b[Ll" will shift the cursor left. "\x1b[Lr" will shift it right.
"\x1b[Lk" will erase the rest of the line.
"\x1b[LI" will initialise the display.
"\x1b[Lx001y001;" will move the cursor to character 001 of line 001. Use any other numbers for different positions. You can also use "\001;" and "\x1b[Ly001;" on their own.
"\x1b[LG0040a0400000000000;" will set up user defined character 00 as a "°" symbol. The first "0" is the character number to define (0-7) and the next 16 characters are hex values for the 8 bytes to define.

*/
