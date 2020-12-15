use std::fs::File;
use std::io::prelude::*;

pub fn get_cpu_temperature() -> i32 {
    let mut file = File::open("/sys/class/thermal/thermal_zone0/temp").unwrap_or_else(|error| {
        panic!(
            "Problem opening the CPU temperature pseudo-file: {:?}",
            error
        );
    });

    let mut cpu_temperature = String::new();

    match file.read_to_string(&mut cpu_temperature) {
        Err(why) => panic!("couldn't read the temperature from the pseduo file {}", why),
        Ok(_file_size) => {
            let milli_temp: i32 = cpu_temperature //cpu_temperature contains the temperature in milli-C and a line terminator
                .trim() //to get rid of the terminator
                .parse()
                .expect("CPU temperature was non-numeric");
            return milli_temp / 1000; //divide by 1000 to convert to C from milli-C and return the temperature
        }
    };
}
