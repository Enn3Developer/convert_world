use convert_world::chunk147;
use fastanvil::{Error, Region};
use rayon::iter::{ParallelBridge, ParallelIterator};
use std::fs::{File, OpenOptions};
use std::io::Read;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, RwLock};
use std::time::Duration;
use std::{cmp, fs, thread};

fn replace_all_old_file(
    path: PathBuf,
    converted_path: PathBuf,
    conversion_map: Arc<RwLock<Vec<(chunk147::Block, chunk147::Block)>>>,
) {
    let file = File::open(path).unwrap();
    let mut mca = Region::from_stream(file).unwrap();
    let mut are_chunks = false;

    if mca.iter().flatten().next().is_some() {
        are_chunks = true;
    }

    if are_chunks {
        let mut open_options = OpenOptions::new();
        open_options
            .write(true)
            .create(true)
            .truncate(true)
            .read(true);
        let converted_file = open_options.open(converted_path).unwrap();
        let mut converted_mca = Region::new(converted_file).unwrap();

        if let Ok(conversion_map) = conversion_map.read() {
            let chunks = mca
                .iter()
                .par_bridge()
                .flatten()
                .map(|chunk| chunk.data)
                .map(|chunk_data| fastnbt::from_bytes::<chunk147::Chunk>(&chunk_data))
                .flatten()
                .map(|mut chunk| {
                    chunk.mut_level().mut_tile_entities().retain(|tile_entity| {
                        tile_entity.id() == "Chest"
                            || tile_entity.id() == "Sign"
                            || tile_entity.id() == "Skull"
                            || tile_entity.id() == "MobSpawner"
                    });

                    for (old_block, new_block) in conversion_map.iter() {
                        for section in chunk.mut_level().mut_sections() {
                            section.replace_all(old_block, new_block);
                        }
                    }

                    chunk
                })
                .collect::<Vec<_>>();

            for chunk in chunks {
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
        }
    }
}

fn read_id(n: &str) -> chunk147::Block {
    if n.contains(':') {
        let mut n_split = n.split(':');
        let id = n_split.next().unwrap().parse::<i32>().unwrap();
        let data = n_split.next().unwrap().parse::<i8>().unwrap();

        chunk147::Block::new(id, Some(data))
    } else {
        chunk147::Block::new(n.parse().unwrap(), None)
    }
}

fn read_conversion_file(
    conversion_path: String,
) -> Arc<RwLock<Vec<(chunk147::Block, chunk147::Block)>>> {
    let mut conversion_map = vec![];
    let mut conversion_file = File::open(conversion_path).unwrap();
    let mut conversion_content = String::new();
    conversion_file
        .read_to_string(&mut conversion_content)
        .unwrap();

    for line in conversion_content.lines() {
        let mut split = line.split("->");
        let o = split.next().unwrap().trim();
        let n = split.next().unwrap().trim();

        let old_id = read_id(o);
        let new_id = read_id(n);

        conversion_map.push((old_id, new_id));
    }

    conversion_map.sort_by(|(a, _), (b, _)| {
        return if a.has_data() && !b.has_data() {
            cmp::Ordering::Less
        } else if b.has_data() && !a.has_data() {
            cmp::Ordering::Greater
        } else {
            if a.id() == b.id() {
                a.data().unwrap().cmp(&b.data().unwrap())
            } else {
                a.id().cmp(&b.id())
            }
        };
    });

    Arc::new(RwLock::new(conversion_map))
}

fn replace_all_old() {
    let is_file = std::env::args().nth(1).unwrap();
    if &is_file == "file" {
        let conversion_path = std::env::args().nth(2).unwrap();
        let path = std::env::args().nth(3).unwrap();
        let converted_path = std::env::args().nth(4).unwrap();

        let conversion_map = read_conversion_file(conversion_path);

        replace_all_old_file(path.into(), converted_path.into(), conversion_map);
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

        let pool = threadpool::Builder::new()
            .num_threads(
                thread::available_parallelism().unwrap().get()
                    * thread::available_parallelism().unwrap().get(),
            )
            .build();
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
                replace_all_old_file(path, converted_path, conversion_map);
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
            if updated == files.len() {
                break;
            }
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
