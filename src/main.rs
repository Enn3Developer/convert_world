use convert_world::chunk147;
use fastanvil::{Error, Region};
use fastnbt::error::Result;
use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::Read;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, RwLock};
use std::time::Duration;
use std::{fs, thread};

// fn convert_to_new() {
//     let path = std::env::args().nth(1).unwrap();
//     let conversion_path = std::env::args().nth(2).unwrap();
//     let converted_path = std::env::args().nth(3).unwrap();
//     let file = File::open(path).unwrap();
//     println!("Opening {}", conversion_path);
//     let mut conversion_file = File::open(conversion_path).unwrap();
//     let mut open_options = OpenOptions::new();
//     open_options
//         .write(true)
//         .create(true)
//         .truncate(true)
//         .read(true);
//     let converted_file = open_options.open(converted_path).unwrap();
//
//     let mut mca = Region::from_stream(file).unwrap();
//     let mut converted_mca = Region::new(converted_file).unwrap();
//     let mut conversion_map = HashMap::new();
//
//     let mut conversion_content = String::new();
//     println!("Reading conversion file");
//     conversion_file
//         .read_to_string(&mut conversion_content)
//         .unwrap();
//
//     for line in conversion_content.lines() {
//         let mut split = line.split("->");
//         let old_id = split
//             .nth(0)
//             .unwrap()
//             .trim()
//             .parse::<chunk147::Block>()
//             .unwrap();
//         let new_id = split.nth(0).unwrap().trim().to_string();
//         conversion_map.insert(old_id, new_id);
//     }
//
//     for (idx, chunk) in mca.iter().flatten().enumerate() {
//         println!("Converted {} chunks", idx);
//         let chunk: Result<chunk147::Chunk> = fastnbt::from_bytes(&chunk.data);
//         let chunk = chunk.unwrap();
//         let new_chunk = chunk1201::Chunk::convert_old(&chunk, &conversion_map);
//         converted_mca
//             .write_chunk(
//                 chunk.level().x_pos() as usize,
//                 chunk.level().z_pos() as usize,
//                 fastnbt::to_bytes(&new_chunk)
//                     .expect("can't convert chunk to bytes")
//                     .as_slice(),
//             )
//             .expect("can't write chunk to file");
//     }
//     println!("Converted all chunks");
// }

enum ChunkMessage {
    JOIN,
    CHUNK(Vec<u8>),
}

fn replace_all_old_file(
    path: String,
    converted_path: String,
    conversion_map: Arc<RwLock<HashMap<chunk147::Block, chunk147::Block>>>,
    data_map: Arc<RwLock<HashMap<chunk147::Block, i8>>>,
) {
    let file = File::open(path.clone()).unwrap();
    let mut open_options = OpenOptions::new();
    open_options
        .write(true)
        .create(true)
        .truncate(true)
        .read(true);
    let converted_file = open_options.open(converted_path.clone()).unwrap();

    // println!("Reading input file {path}");
    let mut mca = Region::from_stream(file).unwrap();
    // println!("Creating output file {converted_path}");
    let mut converted_mca = Region::new(converted_file).unwrap();

    // println!("Initializing data channels");
    let (tx_c, rx_c) = flume::unbounded();
    let (tx_r, rx_r) = flume::unbounded();
    let mut threads = vec![];

    // println!("Initializing chunk threads");
    for _ in 0..thread::available_parallelism().unwrap().get() {
        let rx_c = rx_c.clone();
        let tx_r = tx_r.clone();
        let conversion_map = conversion_map.clone();
        let data_map = data_map.clone();
        threads.push(thread::spawn(move || loop {
            let message: ChunkMessage = rx_c.recv().unwrap();
            if let ChunkMessage::CHUNK(chunk_data) = message {
                let chunk: Result<chunk147::Chunk> = fastnbt::from_bytes(&chunk_data);
                if let Ok(mut chunk) = chunk {
                    for section in chunk.mut_level().mut_sections().iter_mut() {
                        for (key, value) in conversion_map.read().unwrap().iter() {
                            if let Some(data) = data_map.read().unwrap().get(key) {
                                section.replace_all_data(*key, *value, *data);
                            } else {
                                section.replace_all(*key, *value);
                            }
                        }
                    }
                    tx_r.send(chunk).unwrap();
                } else {
                    println!("Error reading chunk");
                }
            } else {
                break;
            }
        }));
    }

    let mut read = 0;

    // println!("Reading chunks");
    for chunk in mca.iter().flatten() {
        read += 1;
        tx_c.send(ChunkMessage::CHUNK(chunk.data)).unwrap();
    }

    let mut converted = 0;

    // println!("Starting receiving converted chunks");
    if read > 0 {
        while let Ok(chunk) = rx_r.recv() {
            converted += 1;
            if let Err(Error::InvalidOffset(x, z)) = converted_mca.write_chunk(
                (chunk.level().x_pos() as usize) % 32,
                (chunk.level().z_pos() as usize) % 32,
                fastnbt::to_bytes(&chunk)
                    .expect("can't convert chunk to bytes")
                    .as_slice(),
            ) {
                println!("can't write chunk at x: {x}, z: {z}");
            }
            if converted == read {
                break;
            }
        }
    }

    // println!("Joining chunk threads");
    for _ in &threads {
        tx_c.send(ChunkMessage::JOIN).unwrap();
    }

    for thread in threads {
        thread.join().unwrap();
    }

    // println!("Converted all chunks");
}

fn read_conversion_file(
    conversion_path: String,
) -> (
    Arc<RwLock<HashMap<chunk147::Block, chunk147::Block>>>,
    Arc<RwLock<HashMap<chunk147::Block, i8>>>,
) {
    let conversion_map = Arc::new(RwLock::new(HashMap::new()));
    let data_map = Arc::new(RwLock::new(HashMap::new()));
    // println!("Opening {}", conversion_path);
    let mut conversion_file = File::open(conversion_path).unwrap();
    let mut conversion_content = String::new();
    // println!("Reading conversion file");
    conversion_file
        .read_to_string(&mut conversion_content)
        .unwrap();

    for line in conversion_content.lines() {
        let mut split = line.split("->");
        let old_id = split
            .nth(0)
            .unwrap()
            .trim()
            .parse::<chunk147::Block>()
            .unwrap();
        let n = split.nth(0).unwrap().trim();
        let new_id = if n.contains(":") {
            let mut n_split = n.split(":");
            let id = n_split.nth(0).unwrap().parse::<chunk147::Block>().unwrap();
            let data = n_split.nth(0).unwrap().parse::<i8>().unwrap();

            data_map.write().unwrap().insert(old_id, data);
            id
        } else {
            n.parse::<chunk147::Block>().unwrap()
        };
        conversion_map.write().unwrap().insert(old_id, new_id);
    }

    (conversion_map, data_map)
}

fn replace_all_old() {
    let is_file = std::env::args().nth(1).unwrap();
    if &is_file == "file" {
        let conversion_path = std::env::args().nth(2).unwrap();
        let path = std::env::args().nth(3).unwrap();
        let converted_path = std::env::args().nth(4).unwrap();

        let (conversion_map, data_map) = read_conversion_file(conversion_path);

        replace_all_old_file(path, converted_path, conversion_map, data_map);
    } else {
        let conversion_path = std::env::args().nth(2).unwrap();
        let dir = std::env::args().nth(3).unwrap();
        let out_dir = std::env::args().nth(4).unwrap();
        let (conversion_map, data_map) = read_conversion_file(conversion_path);
        let mut files = vec![];
        for file in fs::read_dir(dir.clone()).unwrap() {
            let f = file.unwrap();
            if f.file_name().to_str().unwrap().ends_with(".mca") {
                files.push(f.file_name().to_str().unwrap().to_string());
            }
        }

        let pool = threadpool::Builder::new().build();
        let counter = Arc::new(AtomicUsize::new(0));

        for file in &files {
            let mut path = PathBuf::new();
            path.push(dir.clone());
            path.push(file.clone());
            let mut converted_path = PathBuf::new();
            converted_path.push(out_dir.clone());
            converted_path.push(file.clone());

            let conversion_map = conversion_map.clone();
            let data_map = data_map.clone();

            let counter = counter.clone();

            pool.execute(move || {
                replace_all_old_file(
                    path.to_str().unwrap().to_string(),
                    converted_path.to_str().unwrap().to_string(),
                    conversion_map,
                    data_map,
                );
                counter.fetch_add(1, Ordering::SeqCst);
            });
        }

        let mut updated = 0;

        while updated <= files.len() {
            updated = counter.fetch_add(0, Ordering::SeqCst);

            print!("{}% done\r", updated / files.len() * 100);
            thread::sleep(Duration::from_secs(5));
        }

        pool.join();
    }
}

fn main() {
    replace_all_old();
}
