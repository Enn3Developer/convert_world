use convert_world::chunk147;
use fastanvil::{CompressionScheme, Error, Region};
use flate2::bufread::ZlibEncoder;
use flate2::Compression;
use std::cmp;
use std::io::{Cursor, Read};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;
use tikv_jemallocator::Jemalloc;
use tokio::sync::Mutex;
use tokio::task::JoinSet;
use tokio_stream::StreamExt;

#[global_allocator]
static GLOBAL: Jemalloc = Jemalloc;

async fn replace_all_old_file(
    path: PathBuf,
    converted_path: PathBuf,
    conversion_map: Arc<Vec<(chunk147::Block, chunk147::Block)>>,
) {
    let file = tokio::fs::read(path).await.unwrap();
    let mut mca = Region::from_stream(Cursor::new(file)).unwrap();
    let mut are_chunks = false;

    if mca.iter().flatten().next().is_some() {
        are_chunks = true;
    }

    if are_chunks {
        let converted_data = vec![];
        let region = Arc::new(Mutex::new(
            tokio::task::spawn_blocking(move || Region::new(Cursor::new(converted_data)))
                .await
                .unwrap()
                .unwrap(),
        ));

        let mut handles = JoinSet::new();
        for chunk in mca.iter() {
            if let Ok(chunk) = chunk {
                let conversion_map = conversion_map.clone();
                let region = region.clone();
                handles.spawn(async move {
                    let chunk_data = chunk.data;
                    let mut chunk = tokio::task::spawn_blocking(move || {
                        fastnbt::from_bytes::<chunk147::Chunk>(&chunk_data)
                    })
                    .await
                    .unwrap()
                    .unwrap();
                    let tile_entities =
                        tokio_stream::iter(chunk.level().tile_entities().clone().into_iter())
                            .filter(|tile_entity| {
                                tile_entity.id() == "Chest"
                                    || tile_entity.id() == "Sign"
                                    || tile_entity.id() == "Skull"
                                    || tile_entity.id() == "MobSpawner"
                            })
                            .map(|tile_entity| tile_entity.clone())
                            .collect::<Vec<_>>()
                            .await;
                    chunk.mut_level().set_tile_entities(tile_entities);

                    let mut stream = tokio_stream::iter(
                        conversion_map.iter().zip(chunk.mut_level().mut_sections()),
                    );

                    while let Some(((old_block, new_block), section)) = stream.next().await {
                        section.replace_all(old_block, new_block).await;
                    }

                    let mut buf = vec![];
                    let x = (chunk.level().x_pos() as usize) % 32;
                    let z = (chunk.level().z_pos() as usize) % 32;
                    let uncompressed_chunk =
                        fastnbt::to_bytes(&chunk).expect("can't convert chunk to bytes");
                    drop(chunk);
                    let mut enc =
                        ZlibEncoder::new(Cursor::new(uncompressed_chunk), Compression::fast());
                    let buf = tokio::task::spawn_blocking(move || {
                        enc.read_to_end(&mut buf).unwrap();
                        buf
                    })
                    .await
                    .unwrap();
                    let write = region.lock().await.write_compressed_chunk(
                        x,
                        z,
                        CompressionScheme::Zlib,
                        &buf,
                    );

                    if let Err(Error::InvalidOffset(x, z)) = write {
                        println!("can't write chunk at x: {x}, z: {z}");
                    }
                });
            }
        }

        while let Some(_handle) = handles.join_next().await {}

        let region = Arc::into_inner(region).unwrap().into_inner();
        tokio::fs::write(converted_path, region.into_inner().unwrap().into_inner())
            .await
            .unwrap();
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
            return if a.has_data() && !b.has_data() {
                cmp::Ordering::Less
            } else if b.has_data() && !a.has_data() {
                cmp::Ordering::Greater
            } else {
                if a.id() == b.id() && a.has_data() && b.has_data() {
                    a.data().unwrap().cmp(&b.data().unwrap())
                } else {
                    a.id().cmp(&b.id())
                }
            };
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
        let conversion_map = read_conversion_file(conversion_path).await;
        replace_all_old_file(path.into(), converted_path.into(), conversion_map).await;
    } else {
        let conversion_path = std::env::args().nth(2).unwrap();
        let dir = std::env::args().nth(3).unwrap();
        let out_dir = std::env::args().nth(4).unwrap();
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
        let start = Instant::now();
        for _ in 0..(len as f32 / 1000.0).floor() as u32 {
            let mut handles = JoinSet::new();
            let (broadcast, _rx) = tokio::sync::broadcast::channel(1);
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
                        replace_all_old_file(path, converted_path, conversion_map).await;
                    });
                }
                if started % 1000 == 0 {
                    break;
                }
            }

            broadcast.send(true).unwrap();
            while let Some(_handle) = handles.join_next().await {
                i += 1;
                let elapsed = (Instant::now() - start).as_secs_f32();
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
            }
        }

        println!("Done!");
        println!("Took {} seconds", (Instant::now() - start).as_secs_f32());
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
