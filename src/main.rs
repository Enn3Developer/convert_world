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

enum ChunkMessage {
    JOIN,
    CHUNK(Vec<u8>),
}

enum ChunkResult {
    CHUNK(chunk147::Chunk),
    EMPTY,
}

fn replace_all_old_file(
    path: String,
    converted_path: String,
    conversion_map: Arc<RwLock<HashMap<chunk147::Block, chunk147::Block>>>,
) {
    let file = File::open(path.clone()).unwrap();
    let mut open_options = OpenOptions::new();
    open_options
        .write(true)
        .create(true)
        .truncate(true)
        .read(true);
    let converted_file = open_options.open(converted_path.clone()).unwrap();

    let mut mca = Region::from_stream(file).unwrap();
    let mut converted_mca = Region::new(converted_file).unwrap();

    let (tx_c, rx_c) = flume::unbounded();
    let (tx_r, rx_r) = flume::unbounded();
    let mut threads = vec![];

    for _ in 0..thread::available_parallelism().unwrap().get() {
        let rx_c = rx_c.clone();
        let tx_r = tx_r.clone();
        let conversion_map = conversion_map.clone();
        threads.push(thread::spawn(move || loop {
            let message: ChunkMessage = rx_c.recv().unwrap();
            if let ChunkMessage::CHUNK(chunk_data) = message {
                let chunk: Result<chunk147::Chunk> = fastnbt::from_bytes(&chunk_data);
                if let Ok(mut chunk) = chunk {
                    let mut modified = false;
                    for tile_entity in chunk.level().tile_entities() {
                        if !(tile_entity.id() == "Chest"
                            || tile_entity.id() == "Sign"
                            || tile_entity.id() == "Skull"
                            || tile_entity.id() == "MobSpawner")
                        {
                            modified = true;
                        }
                    }

                    if modified {
                        chunk.mut_level().mut_tile_entities().retain(|tile_entity| {
                            tile_entity.id() == "Chest"
                                || tile_entity.id() == "Sign"
                                || tile_entity.id() == "Skull"
                                || tile_entity.id() == "MobSpawner"
                        });
                    }

                    if !modified {
                        if let Ok(conversion_map) = conversion_map.read() {
                            'modified: for section in chunk.level().sections() {
                                for i in 0..section.blocks().len() {
                                    let block = section.blocks()[i];
                                    for old_block in conversion_map.keys() {
                                        if old_block.to_i8() == block {
                                            modified = true;
                                            break 'modified;
                                        }
                                    }
                                }
                            }
                        }
                    }
                    if modified {
                        chunk
                            .mut_level()
                            .mut_sections()
                            .iter_mut()
                            .for_each(|section| {
                                section.replace_all_blocks(conversion_map.clone());
                            });
                        tx_r.send(ChunkResult::CHUNK(chunk)).unwrap();
                    } else {
                        tx_r.send(ChunkResult::EMPTY).unwrap();
                    }
                } else {
                    println!("Error reading chunk");
                }
            } else {
                break;
            }
        }));
    }

    let mut read = 0;

    for chunk in mca.iter().flatten() {
        read += 1;
        tx_c.send(ChunkMessage::CHUNK(chunk.data)).unwrap();
    }

    let mut converted = 0;

    if read > 0 {
        while let Ok(chunk) = rx_r.recv() {
            converted += 1;
            if let ChunkResult::CHUNK(chunk) = chunk {
                if let Err(Error::InvalidOffset(x, z)) = converted_mca.write_chunk(
                    (chunk.level().x_pos() as usize) % 32,
                    (chunk.level().z_pos() as usize) % 32,
                    fastnbt::to_bytes(&chunk)
                        .expect("can't convert chunk to bytes")
                        .as_slice(),
                ) {
                    println!("can't write chunk at x: {x}, z: {z}");
                }
            }
            if converted == read {
                break;
            }
        }
    }

    for _ in &threads {
        tx_c.send(ChunkMessage::JOIN).unwrap();
    }

    for thread in threads {
        thread.join().unwrap();
    }
}

fn read_conversion_file(
    conversion_path: String,
) -> Arc<RwLock<HashMap<chunk147::Block, chunk147::Block>>> {
    let conversion_map = Arc::new(RwLock::new(HashMap::new()));
    let mut conversion_file = File::open(conversion_path).unwrap();
    let mut conversion_content = String::new();
    conversion_file
        .read_to_string(&mut conversion_content)
        .unwrap();

    for line in conversion_content.lines() {
        let mut split = line.split("->");
        let o = split.nth(0).unwrap().trim();
        let old_id = if o.contains(":") {
            let mut n_split = o.split(":");
            let id = n_split.nth(0).unwrap().parse::<i32>().unwrap();
            let data = n_split.nth(0).unwrap().parse::<i8>().unwrap();

            chunk147::Block::new(id, Some(data))
        } else {
            chunk147::Block::new(o.parse().unwrap(), None)
        };
        let n = split.nth(0).unwrap().trim();
        let new_id = if n.contains(":") {
            let mut n_split = n.split(":");
            let id = n_split.nth(0).unwrap().parse::<i32>().unwrap();
            let data = n_split.nth(0).unwrap().parse::<i8>().unwrap();

            chunk147::Block::new(id, Some(data))
        } else {
            chunk147::Block::new(n.parse().unwrap(), None)
        };

        conversion_map.write().unwrap().insert(old_id, new_id);
    }

    conversion_map
}

fn replace_all_old() {
    let is_file = std::env::args().nth(1).unwrap();
    if &is_file == "file" {
        let conversion_path = std::env::args().nth(2).unwrap();
        let path = std::env::args().nth(3).unwrap();
        let converted_path = std::env::args().nth(4).unwrap();

        let conversion_map = read_conversion_file(conversion_path);

        replace_all_old_file(path, converted_path, conversion_map);
    } else {
        let conversion_path = std::env::args().nth(2).unwrap();
        let dir = std::env::args().nth(3).unwrap();
        let out_dir = std::env::args().nth(4).unwrap();
        let conversion_map = read_conversion_file(conversion_path);
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

            let counter = counter.clone();

            pool.execute(move || {
                replace_all_old_file(
                    path.to_str().unwrap().to_string(),
                    converted_path.to_str().unwrap().to_string(),
                    conversion_map,
                );
                counter.fetch_add(1, Ordering::SeqCst);
            });
        }

        let mut updated = 0;
        let mut last_updated = 0;
        let mut time = 0.0;

        while updated < files.len() {
            updated = counter.fetch_add(0, Ordering::SeqCst);
            let mean_rps = if time == 0.0 {
                0.0
            } else {
                updated as f32 / time
            };
            let eta = if mean_rps == 0.0 {
                0.0
            } else {
                (files.len() - updated) as f32 / mean_rps
            };

            print!("\x1B[2J\x1B[1;1H");
            println!(
                "{:.2}% done; {:.2} instantaneous rps; {:.2} mean rps; {}/{}; ETA: {:.1}s",
                (updated as f32) * 100.0 / (files.len() as f32),
                ((updated - last_updated) as f32) / 5.0,
                mean_rps,
                updated,
                files.len(),
                eta
            );
            println!("Made by Enn3DevPlayer");
            println!("Sponsor: N Inc.");
            println!("Special thanks to ChDon for the UI ideas");
            last_updated = updated;
            time += 5.0;
            thread::sleep(Duration::from_secs(5));
        }

        println!("Done!");
        println!("Took {time} seconds");

        pool.join();
    }
}

fn main() {
    replace_all_old();
}
