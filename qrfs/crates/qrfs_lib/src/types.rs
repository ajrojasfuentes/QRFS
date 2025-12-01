use serde::{Deserialize, Serialize};
use std::time::SystemTime;

// --- CONSTANTES DE DISEÑO ---

// Tamaño de un bloque lógico.
// El enunciado sugiere 4KiB, pero un QR de 200x200 (aprox Version 40)
// almacena max ~3KB. Definiremos 1024 bytes para intentar mantener una relación
// 1 bloque lógico = 1 QR físico y simplificar tu vida.
pub const BLOCK_SIZE: usize = 1024; 

// Número mágico para identificar tu FS (como una firma digital simple)
pub const QRFS_MAGIC: u32 = 0x51524653; // Hex para "QRFS" en ASCII

// Longitud máxima del nombre de archivo (simplificación)
pub const MAX_FILENAME_LEN: usize = 64;

// Máximo número de bloques directos en un inodo antes de necesitar indirección
// (Simplificación para el proyecto universitario)
pub const DIRECT_POINTERS: usize = 12; 

// --- ESTRUCTURAS PRINCIPALES ---

/// El Superbloque contiene la información global del sistema de archivos.
/// Esta estructura SIEMPRE va cifrada en el disco[cite: 68].
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct SuperBlock {
    pub magic: u32,             // Debe coincidir con QRFS_MAGIC
    pub total_blocks: u64,      // Tamaño total del FS en bloques [cite: 45]
    pub total_inodes: u64,      // Cantidad total de inodos disponibles [cite: 46]
    pub free_blocks_count: u64, // Contador rápido de espacio libre
    
    // Punteros a áreas críticas (índice del bloque donde empiezan)
    pub inode_table_start: u64, // Dónde empieza la tabla de inodos [cite: 47]
    pub bitmap_start: u64,      // Dónde empieza el mapa de bits [cite: 47]
    pub root_dir_inode: u64,    // Cuál es el inodo de la raíz (usualmente el 1)
    
    // Seguridad
    pub uuid: [u8; 16],         // ID único del volumen
}

/// Tipo de archivo: ¿Es un archivo normal o un directorio?
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Copy)]
pub enum FileType {
    File,
    Directory,
}

/// El Inodo (Index Node) representa un objeto en el FS.
/// Contiene metadatos pero NO el nombre del archivo (eso va en el directorio).
/// [cite: 38, 39, 40]
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Inode {
    pub mode: u16,              // Permisos (ej. 755)
    pub size: u64,              // Tamaño lógico del archivo en bytes
    pub file_type: FileType,    // Archivo o Directorio
    
    // Tiempos (opcionales según enunciado, pero recomendados para FUSE)
    pub created_at: SystemTime,
    pub modified_at: SystemTime,
    
    // Bloques de datos: Lista de IDs de bloques donde está el contenido
    pub direct_blocks: [u64; DIRECT_POINTERS], 
    
    // Para archivos más grandes que (DIRECT_POINTERS * BLOCK_SIZE),
    // usaríamos un bloque indirecto. (Opcional para simplificar si tus archivos son pequeños)
    pub indirect_block: u64, 
}

impl Inode {
    /// Crea un inodo nuevo y vacío
    pub fn new(file_type: FileType, mode: u16) -> Self {
        Self {
            mode,
            size: 0,
            file_type,
            created_at: SystemTime::now(),
            modified_at: SystemTime::now(),
            direct_blocks: [0; DIRECT_POINTERS], // 0 indica "vacío" o "null"
            indirect_block: 0,
        }
    }
}

/// Entrada de Directorio.
/// Como QRFS usa un esquema simple, un directorio es solo un archivo especial
/// cuyo contenido es una lista de estas estructuras.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct DirEntry {
    pub inode_idx: u64,            // A qué inodo apunta
    pub name: String,              // Nombre del archivo ("hola.txt")
}