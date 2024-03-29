use async_stream::stream;
use fastnbt::{ByteArray, IntArray, Value};
use futures_util::pin_mut;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use tokio_stream::StreamExt;

#[derive(Eq, Hash, PartialEq, Clone, Debug)]
pub struct Block {
    id: i32,
    data: Option<i8>,
}

impl Block {
    pub fn new(id: i32, data: Option<i8>) -> Self {
        Self { id, data }
    }

    pub fn to_i8(&self) -> i8 {
        (self.id & 255) as i8
    }

    pub fn add_to_i8(&self) -> i8 {
        ((self.id - (self.id & 0b11111111)) >> 8) as i8
    }

    pub fn from_i8(block: i8) -> Self {
        let mut id = 0i32;

        id |= (block & 0b01111111) as i32;
        id |= (((block >> 7) as i32) & 0b1) << 7;

        Self { id, data: None }
    }

    pub fn has_data(&self) -> bool {
        self.data.is_some()
    }

    pub fn id(&self) -> i32 {
        self.id
    }

    pub fn data(&self) -> Option<i8> {
        self.data
    }
}

#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(rename_all = "PascalCase")]
pub struct Chunk {
    level: Level,
}

impl Chunk {
    pub fn level(&self) -> &Level {
        &self.level
    }
    pub fn mut_level(&mut self) -> &mut Level {
        &mut self.level
    }
}

#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(rename_all = "PascalCase")]
pub struct Level {
    biomes: Option<ByteArray>,
    tile_entities: Vec<TileEntity>,
    height_map: IntArray,
    sections: Vec<Section>,
    last_update: i64,
    #[serde(rename = "xPos")]
    x_pos: i32,
    #[serde(rename = "zPos")]
    z_pos: i32,
    #[serde(flatten)]
    other: HashMap<String, Value>,
}

impl Level {
    pub fn last_update(&self) -> i64 {
        self.last_update
    }
    pub fn biomes(&self) -> Option<&ByteArray> {
        self.biomes.as_ref()
    }
    pub fn mut_tile_entities(&mut self) -> &mut Vec<TileEntity> {
        &mut self.tile_entities
    }
    pub fn set_tile_entities(&mut self, tile_entities: Vec<TileEntity>) {
        self.tile_entities = tile_entities;
    }
    pub fn tile_entities(&self) -> &Vec<TileEntity> {
        &self.tile_entities
    }
    pub fn height_map(&self) -> &IntArray {
        &self.height_map
    }
    pub fn sections(&self) -> &Vec<Section> {
        &self.sections
    }
    pub fn mut_sections(&mut self) -> &mut Vec<Section> {
        &mut self.sections
    }
    pub fn x_pos(&self) -> i32 {
        self.x_pos
    }
    pub fn z_pos(&self) -> i32 {
        self.z_pos
    }
    pub fn other(&self) -> &HashMap<String, Value> {
        &self.other
    }
}

#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(rename_all = "PascalCase")]
pub struct TileEntity {
    #[serde(rename = "id")]
    id: String,
    #[serde(rename = "x")]
    x: i32,
    #[serde(rename = "y")]
    y: i32,
    #[serde(rename = "z")]
    z: i32,

    // GUI Containers
    custom_name: Option<String>,
    // Beacon
    levels: Option<i32>,
    primary: Option<i32>,
    secondary: Option<i32>,
    // Cauldron / Chest / Furnace / Hopper / Trap
    items: Option<Vec<Value>>,
    // Cauldron
    brew_time: Option<i32>,
    // Control
    command: Option<String>,
    // Furnace
    burn_time: Option<i16>,
    cook_time: Option<i16>,
    // Mob Spawner
    entity_id: Option<String>,
    spawn_count: Option<i16>,
    spawn_range: Option<i16>,
    spawn_data: Option<Value>,
    spawn_potentials: Option<Vec<Value>>,
    delay: Option<i16>,
    min_spawn_delay: Option<i16>,
    max_spawn_delay: Option<i16>,
    max_nearby_entities: Option<i16>,
    required_player_range: Option<i16>,
    max_experience: Option<i32>,
    remaining_experience: Option<i32>,
    experience_regen_tick: Option<i32>,
    experience_regen_rate: Option<i32>,
    experience_regen_amount: Option<i32>,
    // Music
    #[serde(rename = "note")]
    note: Option<i8>,
    // Piston
    #[serde(rename = "blockId")]
    block_id: Option<i32>,
    #[serde(rename = "blockData")]
    block_data: Option<i32>,
    #[serde(rename = "facing")]
    facing: Option<i32>,
    #[serde(rename = "progress")]
    progress: Option<f32>,
    #[serde(rename = "extending")]
    extending: Option<i8>,
    // Record Player
    record: Option<i32>,
    record_item: Option<Value>,
    // Sign
    text1: Option<String>,
    text2: Option<String>,
    text3: Option<String>,
    text4: Option<String>,
    // Skull
    skull_type: Option<i8>,
    extra_type: Option<String>,
    rot: Option<i8>,
    #[serde(flatten)]
    other: HashMap<String, Value>,
}

impl TileEntity {
    pub fn id(&self) -> &String {
        &self.id
    }

    pub fn other(&self) -> &HashMap<String, Value> {
        &self.other
    }
}

#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(rename_all = "PascalCase")]
pub struct Section {
    blocks: ByteArray,
    add: Option<ByteArray>,
    y: i8,
    data: Option<ByteArray>,
    #[serde(flatten)]
    other: HashMap<String, Value>,
}

impl Section {
    pub fn y(&self) -> i8 {
        self.y
    }

    pub fn blocks(&self) -> &ByteArray {
        &self.blocks
    }

    pub fn add(&self) -> Option<&ByteArray> {
        self.add.as_ref()
    }

    pub fn other(&self) -> &HashMap<String, Value> {
        &self.other
    }

    pub fn block(&self, x: i32, y: i32, z: i32) -> Block {
        let block_pos = (y * 16 * 16 + z * 16 + x) as usize;
        let mut block = self.blocks[block_pos] as i32;
        if let Some(added) = &self.add {
            block += ((added[block_pos / 2] as i32) >> (4 * (block_pos % 2))) << 8;
        }
        let mut data = None;
        if let Some(data_arr) = &self.data {
            let d = if block_pos % 2 == 0 {
                data_arr[block_pos / 2] & 0x0F
            } else {
                (data_arr[block_pos / 2] >> 4) & 0x0F
            };
            data = Some(d);
        }

        Block { id: block, data }
    }

    pub async fn replace_all_blocks(
        &mut self,
        conversion_map: Arc<RwLock<HashMap<Block, Block>>>,
    ) -> bool {
        let mut conversion = Vec::with_capacity(1024);
        for (idx, block) in self.blocks.iter().enumerate() {
            let mut b = Block::from_i8(*block);

            let d = if let Some(added) = &self.add {
                if idx % 2 == 0 {
                    added[idx / 2] & 0x0F
                } else {
                    (added[idx / 2] >> 4) & 0x0F
                }
            } else {
                0
            };
            b.id += (d as i32) << 8;
            if let Some(data_arr) = &self.data {
                let d = if idx % 2 == 0 {
                    data_arr[idx / 2] & 0x0F
                } else {
                    (data_arr[idx / 2] >> 4) & 0x0F
                };
                b.data = Some(d);
            }
            if let Ok(conversion_map) = conversion_map.read() {
                if let Some(new_block) = conversion_map.get(&b) {
                    conversion.push((idx, new_block.clone()));
                } else {
                    b.data = None;
                    if let Some(new_block) = conversion_map.get(&b) {
                        conversion.push((idx, new_block.clone()));
                    }
                }
            }
        }

        for (idx, block) in &conversion {
            self.replace_block(*idx, block).await;
        }

        !conversion.is_empty()
    }

    fn get_data(idx: usize, arr: Option<&ByteArray>) -> i8 {
        if let Some(data) = arr {
            if idx % 2 == 0 {
                data[idx / 2] & 0x0F
            } else {
                (data[idx / 2] >> 4) & 0x0F
            }
        } else {
            0
        }
    }

    pub async fn replace_all(&mut self, old: &Block, new: &Block) {
        let blocks = self.blocks.clone();
        let blocks = blocks.iter().enumerate();
        let data = self.data.clone();
        let data_ref = data.as_ref();
        let add = self.add.clone();
        let add_ref = add.as_ref();
        let test = stream! {
            for (idx, block) in blocks {
                if *block == old.to_i8() {
                    let data = Self::get_data(idx, data_ref);
                    let d = Self::get_data(idx, add_ref);

                    if old.data.is_none() || old.data.unwrap() == data {
                        if old.id < 256 && d == 0 {
                            yield (idx, new);
                        } else {
                            let b = Block::from_i8(*block);
                            let id = b.id + ((d as i32) << 8);
                            if id == old.id {
                                yield (idx, new);
                            }
                        }
                    }
                }
            }
        };

        pin_mut!(test);

        while let Some((idx, block)) = test.next().await {
            self.replace_block(idx, block).await;
        }
    }

    pub async fn replace_block(&mut self, idx: usize, new_block: &Block) {
        if let Some(new_data) = new_block.data {
            if let Some(data_arr) = &mut self.data {
                let d = if idx % 2 == 0 {
                    data_arr[idx / 2] & 0x0F
                } else {
                    (data_arr[idx / 2] >> 4) & 0x0F
                };
                data_arr[idx / 2] -= d << (4 * (idx % 2));
                data_arr[idx / 2] += new_data << (4 * (idx % 2));
            }
        }

        if let Some(added) = &mut self.add {
            let d = if idx % 2 == 0 {
                added[idx / 2] & 0x0F
            } else {
                (added[idx / 2] >> 4) & 0x0F
            };
            added[idx / 2] -= d << (4 * (idx % 2));
            added[idx / 2] += new_block.add_to_i8() << (4 * (idx % 2));
        }

        self.blocks[idx] = new_block.to_i8();
    }
}
