use tokio::net::TcpListener;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use crc::{Crc, CRC_32_ISCSI};
use prost::bytes::BytesMut;
use prost::encoding::group::encode_repeated;
use prost::Message;
use std::fs::File;
use std::io::prelude::*;
use chrono::{Datelike, Timelike, Utc};
use http::request::Request;

include!("../protocols/messages.rs");
pub const X25: Crc<u32> = Crc::<u32>::new(&CRC_32_ISCSI);

#[derive(Debug)]
pub struct Packet {
    api_version: u8,
    flag: u16,
    message_type: u16,
    data_length: u64,
    pub data: Vec<u8>,
    crc: u16,
}

impl Packet {
    fn new() -> Self {
        Self {
            api_version: 0,
            flag: 0,
            message_type: 0,
            data_length: 0,
            data: Vec::with_capacity(4),
            crc: 0,
        }
    }
    fn from(
        api_version: u8,
        flag: u16,
        message_type: u16,
        data_length: u64,
        data: Vec<u8>,
        crc: u16,
    ) -> Self {
        Self {
            api_version,
            flag,
            message_type,
            data_length,
            data,
            crc,
        }
    }
}
fn pb_header_checker(buf: &Vec<u8>) -> Result<Packet, ()> {
    if buf.len() < 11 {
        return Err(());
    }
    //check splitter
    let mut iter = buf.iter();
    let start_index = iter.position(|&x| x == 143).unwrap();
    if *iter.next().unwrap() != 179 {
        return Err(());
    }
    let mut data: u64 = 0;
    let mut dat: u8;
    let mut res = Vec::with_capacity(3);
    for i in &[1, 2, 2, 4] {
        for _ in 0..*i {
            dat = *iter.next().unwrap();
            data = (data << 8) | dat as u64;
            dat = 00;
        }
        res.push(data);
        data = 0;
    }
    if res[3] > iter.size_hint().0 as u64 {
        return Err(());
    }
    //extract messages
    //let mut messages =
    //    Vec::from(&buf[(start_index + 11)..(start_index + 11 as usize + res[3] as usize)]);
    let mut messages = Vec::with_capacity(res[3] as usize);
    for _ in 0..res[3] {
        let dat = *iter.next().unwrap();
        messages.push(dat);
    }
    //extract crc
    dat = 0;
    for _ in 0..4 {
        dat = *iter.next().unwrap();
        data = (data << 8) | dat as u64;
    }
    // crc checking
    if data != X25.checksum(&messages[..]) as u64 {
        return Err(());
    }
    Ok(Packet::from(
        res[0] as u8,
        res[1] as u16,
        res[2] as u16,
        res[3] as u64,
        messages,
        0,
    ))
}
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let listener = TcpListener::bind("127.0.0.1:9595").await?;

    loop {
        let (mut socket, _) = listener.accept().await?;

        tokio::spawn(async move {
            let mut buf = [0; 1024*100];

            // In a loop, read data from the socket and write the data back.
            loop {
                let n = match socket.read(&mut buf).await {
                    // socket closed
                    Ok(n) if n == 0 => return,
                    Ok(n) => n,
                    Err(e) => {
                        eprintln!("failed to read from socket; err = {:?}", e);
                        return;
                    }
                };
                let now = Utc::now();
                let (is_pm, hour) = now.hour12();

                let mut file = File::create(format!(
                    "Point_{:02}_{:02}_{:02}_{:02}_{:02}_{}",
                    now.month(),
                    now.day(),
                    hour,
                    now.minute(),
                    now.second(),
                    if is_pm { "PM" } else { "AM" }
                )).unwrap();
                let packet = match pb_header_checker(&buf.to_vec()) {
                    Ok(msg) =>{ println!("[PH] Packet Received \r\n");
                        msg},
                    Error => Packet::new(),
                };
                let datapack = DataPointList::decode(
                    BytesMut::from(&packet.data[..])).unwrap();
                if datapack.len() > 0 {
                    let res = reqwest::get(
                        format!("http://location.lagra.ir/addloc.php?name=farbod&lat={}&lang={}",
                                datapack.fields_list[0].latitude
                                , datapack.fields_list[0].longitude)).await;

                    println!("Status: {}", res.status());
                    let body = res.text().await;

                    println!("Body:\n\n{}", body);
                }

                let serialized_user =
                    serde_json::to_string(&datapack).unwrap();
                println!("{}", serialized_user);
                file.write_all(serialized_user.as_bytes());
                file.sync_data();
                // Write the data back
                if let Err(e) = socket.write_all(&buf[0..n]).await {
                    eprintln!("failed to write to socket; err = {:?}", e);
                    return;
                }
            }
        });
    }
}