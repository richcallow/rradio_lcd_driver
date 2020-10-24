use std::io::Read;

fn main() {
    println!("Hello, world!");
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

        println!("length {},   {:?}", message_length, buffer);
    }
}
