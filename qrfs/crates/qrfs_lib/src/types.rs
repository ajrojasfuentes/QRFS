use serde::{Deserialize, Serialize};
use std::time::SystemTime;

// --- CONSTANTES DE DISEÑO ---
pub const BLOCK_SIZE: usize = 1024; 
pub const QRFS_MAGIC: u32 = 0x51524653;
pub const MAX_FILENAME_LEN: usize = 64;

// NOTA: Eliminamos DIRECT_POINTERS fijo. Ahora vive en el Superbloque.

// --- ESTRUCTURAS PRINCIPALES ---

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct SuperBlock {
    pub magic: u32,
    pub total_blocks: u64,
    pub total_inodes: u64,
    pub free_blocks_count: u64,
    
    pub inode_table_start: u64,
    pub bitmap_start: u64,
    pub root_dir_inode: u64,
    
    pub uuid: [u8; 16],

    // NUEVO: Configuración de geometría dinámica
    // Esto le dice a 'mount' qué tan grandes son los inodos en este disco
    pub direct_pointers_count: u32, 
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Copy)]
pub enum FileType {
    File,
    Directory,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Inode {
    pub mode: u16,
    pub size: u64,
    pub file_type: FileType,
    pub created_at: SystemTime,
    pub modified_at: SystemTime,
    
    // CAMBIO CRÍTICO: De array fijo [u64; 12] a Vector dinámico
    // Esto permite que el inodo crezca o se encoja según la configuración.
    pub direct_blocks: Vec<u64>, 
    
    pub indirect_block: u64, 
}

impl Inode {
    // Ahora necesitamos saber cuántos punteros asignar al crearlo
    pub fn new(file_type: FileType, mode: u16, num_pointers: u32) -> Self {
        Self {
            mode,
            size: 0,
            file_type,
            created_at: SystemTime::now(),
            modified_at: SystemTime::now(),
            // Inicializamos el vector con ceros
            direct_blocks: vec![0; num_pointers as usize], 
            indirect_block: 0,
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct DirEntry {
    pub inode_idx: u64,
    pub name: String,
}