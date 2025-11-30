use clap::Parser;
use qrfs_lib::device::BlockDevice;
use qrfs_lib::types::{SuperBlock, Inode, FileType, BLOCK_SIZE, DIRECT_POINTERS, QRFS_MAGIC};
use qrfs_lib::bitmap::Bitmap;
use qrfs_lib::crypto::CryptoEngine;
use std::path::PathBuf;
use std::io::Write;
use rpassword::read_password;

/// Herramienta para formatear un sistema de archivos QRFS
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Ruta al directorio donde se guardarán los QRs
    #[arg(value_name = "QR_FOLDER")]
    path: PathBuf,

    /// Número de bloques a crear (si no existen ya)
    #[arg(short, long, default_value_t = 100)]
    blocks: u64,
}

fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    println!("=== Formateador QRFS ===");
    println!("Directorio objetivo: {:?}", args.path);

    // 1. Inicializar dispositivo
    let device = BlockDevice::new(&args.path)?;
    
    // Si la carpeta está vacía, podríamos pre-generar los bloques físicos,
    // pero QRFS los creará on-demand al escribir. Validamos el tamaño.
    let total_blocks = args.blocks;
    if total_blocks < 5 {
        anyhow::bail!("El tamaño mínimo es de 5 bloques (Superbloque + Bitmap + Inodos + Raíz + Datos)");
    }

    // 2. Pedir contraseña
    print!("Ingrese la passphrase para cifrar el sistema: ");
    std::io::stdout().flush()?;
    let password = read_password()?;
    
    print!("Confirme la passphrase: ");
    std::io::stdout().flush()?;
    let confirm = read_password()?;

    if password != confirm {
        anyhow::bail!("Las contraseñas no coinciden.");
    }

    // 3. Inicializar Criptografía (Genera un Salt aleatorio nuevo)
    let crypto = CryptoEngine::new_with_random_salt(&password);

    println!("Iniciando formateo de {} bloques...", total_blocks);

    // --- CÁLCULO DE ESTRUCTURA ---
    // Distribución simple:
    // Bloque 0: Header (Salt) + Superbloque Cifrado
    // Bloque 1: Bitmap (Cifrado)
    // Bloque 2..N: Tabla de Inodos (Cifrada)
    // Bloque N+1..: Datos

    // Reservamos espacio para tabla de inodos (ej. 10% del disco o fijo)
    // Simplificación: 10 inodos por bloque. Digamos que queremos soportar 'total_blocks' archivos.
    // Tamaño inode = aprox 120 bytes. En 1KB caben unos 8.
    // Reservamos (total_blocks / 8) bloques para inodos.
    let inode_blocks = (total_blocks / 8).max(1); 
    let bitmap_blocks = 1; // Para 100 bloques sobra con 1 bloque de bitmap (maneja 8192 bloques)
    
    let sb_idx = 0;
    let bitmap_idx = 1;
    let inode_table_idx = 2;
    let data_start_idx = inode_table_idx + inode_blocks;

    let total_inodes = inode_blocks * (BLOCK_SIZE as u64 / 128); // Estimado grosero

    // 4. Crear Estructuras en Memoria

    // A) BITMAP
    let mut bitmap = Bitmap::new(total_blocks as usize);
    // Marcar bloques de sistema como ocupados
    for i in 0..data_start_idx {
        bitmap.set(i as usize, true);
    }
    // Marcar el primer bloque de datos como ocupado (para el directorio raíz)
    let root_block = data_start_idx;
    bitmap.set(root_block as usize, true);

    // B) INODO RAÍZ
    let mut root_inode = Inode::new(FileType::Directory, 0o755);
    root_inode.size = 0; // El tamaño crece conforme metemos DirEntries
    root_inode.direct_blocks[0] = root_block; // Apunta al primer bloque de datos reservado
    
    // C) SUPERBLOQUE
    let sb = SuperBlock {
        magic: QRFS_MAGIC,
        total_blocks,
        total_inodes,
        free_blocks_count: total_blocks - data_start_idx - 1,
        inode_table_start: inode_table_idx,
        bitmap_start: bitmap_idx,
        root_dir_inode: 1, // El inodo 1 será la raíz (el 0 suele ser nulo)
        uuid: *uuid::Uuid::new_v4().as_bytes(),
    };

    // 5. Escritura en Disco (Física + Cifrado)

    // PASO 1: Escribir Superbloque (Bloque 0)
    // Formato especial: [SALT (16 bytes)] [ENCRYPTED_DATA]
    let sb_bytes = bincode::serialize(&sb)?;
    let sb_encrypted = crypto.encrypt(&sb_bytes)?;
    
    let mut block0_data = Vec::new();
    block0_data.extend_from_slice(&crypto.salt); // Guardamos Salt en claro
    block0_data.extend_from_slice(&sb_encrypted);
    
    let mut block0_data = Vec::new();
    block0_data.extend_from_slice(&crypto.salt); // Guardamos Salt en claro
    block0_data.extend_from_slice(&sb_encrypted);
    
    // --- BORRA O COMENTA ESTO ---
    // if block0_data.len() < BLOCK_SIZE {
    //    block0_data.resize(BLOCK_SIZE, 0);
    // }
    // ----------------------------
    
    device.write_block(sb_idx, &block0_data)?;
    
    device.write_block(sb_idx, &block0_data)?;
    println!("[x] Superbloque escrito en bloque {}", sb_idx);

    // PASO 2: Escribir Bitmap
    let bitmap_bytes = bincode::serialize(&bitmap)?;
    let bitmap_encrypted = crypto.encrypt(&bitmap_bytes)?;
    // Nota: Si el bitmap cifrado excede 1 bloque, esto fallará en device. 
    // Para el proyecto, asumimos discos pequeños (<8000 bloques).
    device.write_block(bitmap_idx, &bitmap_encrypted)?;
    println!("[x] Bitmap escrito en bloque {}", bitmap_idx);

    // PASO 3: Escribir Tabla de Inodos
    // El Inodo Raíz (índice 1) vive en el primer bloque de la tabla de inodos.
    // Calculamos cuántos inodos caben en un bloque para no pasarnos.
    
    // Un inodo serializado pesa aprox 120-150 bytes.
    // En 1024 bytes caben unos 6-8 inodos.
    let inodes_per_block = (BLOCK_SIZE / 150).max(1); 
    
    // Creamos SOLO el primer paquete de inodos
    let mut first_inode_block = vec![Inode::new(FileType::File, 0); inodes_per_block];
    first_inode_block[1] = root_inode; // Inodo 1 es Root

    let inodes_bytes = bincode::serialize(&first_inode_block)?;
    
    // Verificación de seguridad antes de cifrar
    if inodes_bytes.len() > BLOCK_SIZE - 64 { // Margen para overhead de cifrado
        anyhow::bail!("Error crítico: Los inodos no caben en el bloque. Reduce inodes_per_block.");
    }

    let inodes_encrypted = crypto.encrypt(&inodes_bytes)?;
    
    // Escribimos SOLO el primer bloque de la tabla (donde está la raíz)
    device.write_block(inode_table_idx, &inodes_encrypted)?;
    println!("[x] Tabla de inodos (bloque inicial) escrita en bloque {}", inode_table_idx);

    // PASO 4: Escribir el directorio raíz (Datos)
    // El inodo raíz apunta a `root_block`. Debe contener una lista vacía de archivos.
    let empty_dir: Vec<qrfs_lib::types::DirEntry> = Vec::new();
    let dir_bytes = bincode::serialize(&empty_dir)?;
    let dir_encrypted = crypto.encrypt(&dir_bytes)?;
    device.write_block(root_block, &dir_encrypted)?;
    println!("[x] Directorio raíz inicializado en bloque {}", root_block);

    println!("¡Formateo completado exitosamente!");
    Ok(())
}