use std::fs::File;
use std::io::prelude::Read; //needed for .read_to_string

pub fn get_wifi_signal_strength() -> String {
    let mut file = match File::open("/proc/net/wireless") {
        Ok(file) => file,
        Err(error) => {
            println!(
                "Problem opening the signal strength pseudo-file: {:?}",
                error
            );
            return "er1".to_string();
        }
    };

    let mut signal_strength = String::new();

    match file.read_to_string(&mut signal_strength) {
        Err(why) => {
            return format!(
                "couldn't read the signal strength from the pseduo file {}",
                why
            )
        }
        Ok(_file_size) => {
            if let Some(position_wlan) = signal_strength.find("wlan") {
                let wlan_line = signal_strength.split_at(position_wlan + "wlan0".len()).1; // the contents of the line containing eg wlan0:
                if let Some(position_minus) = wlan_line.find("-") {
                    let level_and_other_characters = wlan_line.split_at(position_minus).1; // level is always negative
                    if let Some(position_dot) = level_and_other_characters.find(".") {
                        let level = level_and_other_characters.split_at(position_dot).0;
                        return level.to_string();
                    }
                }
            }
            return "er2".to_string();
        }
    };
}
