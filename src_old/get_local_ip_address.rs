pub fn get_local_ip_address() -> String {
    let mut return_value: String = String::from("bad Local IP address");
    let mut got_address = false;

    for _ip_count in 1..100 {
        // go round a wait loop many timesas we need t owait for the IP addresses
        // many times before we get the local IP address working
        for iface in pnet::datalink::interfaces() {
            if iface.is_up() && !iface.is_loopback() && iface.ips.len() > 0 {
                // this if statement filters off the loopback address & addresses that do not have an IP address
                for ipaddr in &iface.ips {
                    let ip4addr = match ipaddr {
                        pnet::ipnetwork::IpNetwork::V4(addr) => addr.ip(), // filters off the "/24" at the end of the IP address
                        pnet::ipnetwork::IpNetwork::V6(_) => continue,
                    };
                    return_value = ip4addr.to_string();
                    got_address = true;
                }
            }
            if got_address {
                break;
            }
        }
        if got_address {
            break;
        }
        use std::thread::sleep;
        use std::time::Duration;
        sleep(Duration::from_millis(250)); //sleep until the Ethernet interface is up
    }
    return_value
}
