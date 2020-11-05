use anyhow::Context;
use rradio_messages::PipelineState;
use std::io::Read;

mod get_local_ip_address;
mod get_temperature;
mod lcd_screen;

type Event = rradio_messages::Event<String, String, Vec<rradio_messages::Track>>;

fn main() -> Result<(), anyhow::Error> {
    let mut lcd = lcd_screen::LcdScreen::new()
        .context("Failed to initialize LCD screen")
        .map_err(|err| {
            // Kill other screen drivers here
            err
        })?;

    lcd.write_ascii(
        lcd_screen::LCDLineNumbers::Line1,
        0,
        get_local_ip_address::get_local_ip_address().as_str(),
    );
    lcd.write_ascii(
        lcd_screen::LCDLineNumbers::Line4,
        0,
        &format!("CPU Temp {} C", get_temperature::get_cpu_temperature()),
    );
    lcd.write_multiline(
        lcd_screen::LCDLineNumbers::Line2,
        40,
        "test22éè123456789012345678901234567890",
    );

    let mut pipe_line_state: PipelineState = PipelineState::VoidPending;
    let mut volume: i32 = -1;
    let mut current_track_index: usize = 0;

    let mut connection =
        std::net::TcpStream::connect((std::net::Ipv4Addr::LOCALHOST, 8002)).unwrap();

    loop {
        let mut message_length_buffer = [0; 2];
        match connection.read(&mut message_length_buffer).unwrap() {
            0 => break,
            2 => (),
            _ => panic!("Weird number of bytes read"),
        }

        let message_length = u16::from_be_bytes(message_length_buffer);

        let mut buffer = vec![0; message_length as usize];

        connection.read_exact(&mut buffer).unwrap();

        //println!("length {},   {:?}", message_length, buffer);

        let event: Event = rmp_serde::from_slice(&buffer).unwrap();

        // println!("Event: {:?}", event);

        match event {
            Event::ProtocolVersion(version) => assert_eq!(version, rradio_messages::VERSION),
            Event::LogMessage(log_message) => println!("{:?}", log_message),
            Event::PlayerStateChanged(diff) => {
                if let Some(pipeline_state) = diff.pipeline_state {
                    pipe_line_state = pipeline_state;
                    lcd.write_volume(pipe_line_state, volume);
                }
                if let Some(current_station) = diff.current_station.into_option() {
                    if let Some(station) = current_station {
                        println!("Current Station{:?}", station);
                    }
                }
                if let Some(current_track_index_in) = diff.current_track_index {
                    current_track_index = current_track_index_in;
                    println!("Current Track index: {}", current_track_index);
                }
                if let Some(current_track_tags) = diff.current_track_tags.into_option() {
                    println!("Current Track Tags: {:?}", current_track_tags);
                }
                if let Some(volume_in) = diff.volume {
                    volume = volume_in;
                    lcd.write_volume(pipe_line_state, volume);
                }
                if let Some(buffering) = diff.buffering {
                    lcd.write_buffer_state(buffering);
                }
                if let Some(track_duration) = diff.track_duration.into_option() {
                    println!("track duration: {:?}", track_duration);
                }
                if let Some(track_position) = diff.track_position.into_option() {
                    //println!("track position: {:?}", track_position);
                }
            }
        }
    }
    lcd.clear();
    lcd.write_ascii(lcd_screen::LCDLineNumbers::Line1, 0, "Ending screen driver");
    lcd.write_multiline(
        lcd_screen::LCDLineNumbers::Line3,
        40,
        "Computer not shut   down",
    );
    println!("exiting screen driver");

    Ok(())
}
