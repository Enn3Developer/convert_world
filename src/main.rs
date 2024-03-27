use async_compression::Level;
use convert_world::chunk147;
use convert_world::region::Region;
use fastanvil::Error;
use std::cmp;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;
use tikv_jemallocator::Jemalloc;
use tokio::fs::{File, OpenOptions};
use tokio::task::JoinSet;
use tokio_stream::StreamExt;

#[global_allocator]
static GLOBAL: Jemalloc = Jemalloc;

async fn replace_all_old_file(
    path: PathBuf,
    converted_path: PathBuf,
    conversion_map: Arc<Vec<(chunk147::Block, chunk147::Block)>>,
    compression: Level,
) {
    let file = File::open(path).await.unwrap();
    let mut mca = Region::from_async_stream(file).await.unwrap();

    if mca.iter().next().await.is_none() {
        return;
    }

    let mut handles = JoinSet::new();
    let mut mca_stream = mca.iter();

    while let Some(chunk) = mca_stream.next().await {
        if let Ok(chunk) = chunk {
            let conversion_map = conversion_map.clone();
            handles.spawn(async move {
                let mut chunk = fastnbt::from_bytes::<chunk147::Chunk>(&chunk.data).unwrap();

                for (old, new) in conversion_map.iter() {
                    for section in chunk.mut_level().mut_sections() {
                        section.replace_all(old, new).await;
                    }
                }

                chunk
            });
        }
    }

    let converted_file = OpenOptions::new()
        .write(true)
        .read(true)
        .create(true)
        .truncate(true)
        .open(converted_path)
        .await
        .unwrap();
    let mut region = Region::async_new(converted_file).await.unwrap();
    while let Some(Ok(chunk)) = handles.join_next().await {
        let x = (chunk.level().x_pos() as usize) % 32;
        let z = (chunk.level().z_pos() as usize) % 32;
        if let Err(Error::InvalidOffset(x, z)) = region
            .async_write_chunk_with_quality(
                x,
                z,
                &fastnbt::to_bytes(&chunk).expect("can't convert chunk to bytes"),
                compression,
            )
            .await
        {
            println!("can't write chunk at x: {x}, z: {z}");
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

async fn read_conversion_file(
    conversion_path: String,
) -> Arc<Vec<(chunk147::Block, chunk147::Block)>> {
    let mut conversion_map = vec![];
    let conversion_content = tokio::fs::read_to_string(conversion_path).await.unwrap();

    for line in conversion_content.lines() {
        let mut split = line.split("->");
        let o = split.next().unwrap().trim();
        let n = split.next().unwrap().trim();

        let old_id = read_id(o);
        let new_id = read_id(n);

        conversion_map.push((old_id, new_id));
    }

    let conversion_map = tokio::task::spawn_blocking(move || {
        conversion_map.sort_by(|(a, _), (b, _)| {
            if a.has_data() && !b.has_data() {
                cmp::Ordering::Less
            } else if b.has_data() && !a.has_data() {
                cmp::Ordering::Greater
            } else if a.id() == b.id() && a.has_data() && b.has_data() {
                a.data().unwrap().cmp(&b.data().unwrap())
            } else {
                a.id().cmp(&b.id())
            }
        });
        conversion_map
    })
    .await
    .unwrap();

    Arc::new(conversion_map)
}

async fn replace_all_old() {
    let is_file = std::env::args().nth(1).unwrap();
    if &is_file == "file" {
        let conversion_path = std::env::args().nth(2).unwrap();
        let path = std::env::args().nth(3).unwrap();
        let converted_path = std::env::args().nth(4).unwrap();
        let compression = std::env::args().nth(5).unwrap_or(String::from("fast"));
        let compression = if &compression == "fast" {
            Level::Fastest
        } else if &compression == "best" {
            Level::Best
        } else {
            Level::Default
        };
        let conversion_map = read_conversion_file(conversion_path).await;
        replace_all_old_file(
            path.into(),
            converted_path.into(),
            conversion_map,
            compression,
        )
        .await;
    } else {
        let conversion_path = std::env::args().nth(2).unwrap();
        let dir = std::env::args().nth(3).unwrap();
        let out_dir = std::env::args().nth(4).unwrap();
        let compression = std::env::args().nth(5).unwrap_or(String::from("fast"));
        let compression = if &compression == "fast" {
            Level::Fastest
        } else if &compression == "best" {
            Level::Best
        } else {
            Level::Default
        };
        let max_workers = std::env::args()
            .nth(6)
            .unwrap_or(String::from("8192"))
            .parse::<u32>()
            .unwrap();
        let conversion_map = read_conversion_file(conversion_path).await;
        let mut read_dir = tokio::fs::read_dir(dir.clone()).await.unwrap();
        println!("Starting all workers");
        let mut started = 0;
        let mut len = 0;
        {
            let mut read_dir = tokio::fs::read_dir(dir.clone()).await.unwrap();
            while let Ok(Some(_)) = read_dir.next_entry().await {
                len += 1;
            }
        }

        let mut i = 0;
        let (broadcast, _rx) = tokio::sync::broadcast::channel(1);
        let mut pauses = 0.0;
        let start = Instant::now();
        for _ in 0..(len as f32 / max_workers as f32).ceil() as u32 {
            let start_pause = Instant::now();
            let mut handles = JoinSet::new();
            while let Ok(Some(file)) = read_dir.next_entry().await {
                let name = file.file_name().clone();
                let name = name.to_string_lossy().to_string();
                let dir = dir.clone();
                let out_dir = out_dir.clone();
                let conversion_map = conversion_map.clone();
                if name.ends_with(".mca") {
                    started += 1;
                    let mut rx = broadcast.subscribe();
                    handles.spawn(async move {
                        let _ = rx.recv().await.unwrap();
                        let mut path = PathBuf::new();
                        path.push(dir);
                        path.push(&name);
                        let mut converted_path = PathBuf::new();
                        converted_path.push(out_dir);
                        converted_path.push(name);
                        replace_all_old_file(path, converted_path, conversion_map, compression)
                            .await;
                    });
                    if started % max_workers == 0 {
                        break;
                    }
                }
            }
            pauses += (Instant::now() - start_pause).as_secs_f32();

            println!("Signaling all workers");
            broadcast.send(true).unwrap();
            while let Some(_handle) = handles.join_next().await {
                i += 1;
                let now = Instant::now();
                let elapsed = (now - start).as_secs_f32() - pauses;
                let mean_rps = if elapsed == 0.0 {
                    0.0
                } else {
                    i as f32 / elapsed
                };
                let eta = if mean_rps == 0.0 {
                    0.0
                } else {
                    (len - i) as f32 / mean_rps
                };

                tokio::task::spawn_blocking(move || {
                    print!("\x1B[2J\x1B[1;1H");
                    println!(
                    "{:.2}% done; {:.2} mean rps; {}/{}; ETA: {:.1}s; elapsed: {:.1}s; started workers: {started}",
                    (i as f32) * 100.0 / (len as f32),
                    mean_rps,
                    i,
                    len,
                    eta,
                    elapsed
                );
                    println!("Made by Enn3DevPlayer");
                    println!("Sponsor: N Inc.");
                    println!("Special thanks to ChDon for the UI ideas");
                }).await.unwrap();
            }
        }

        println!("Done!");
        println!(
            "Took {} seconds",
            (Instant::now() - start).as_secs_f32() - pauses
        );
    }
}

async fn run() {
    replace_all_old().await;
}

fn main() {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(run())
}
