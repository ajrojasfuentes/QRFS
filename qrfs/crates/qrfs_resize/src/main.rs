use clap::Parser;
use std::path::PathBuf;
use std::io::Write;
use rpassword::read_password;
use colored::*;

use qrfs_lib::device::BlockDevice;
use qrfs_lib::crypto::CryptoEngine;
use qrfs_lib::types::{SuperBlock, QRFS_MAGIC, BLOCK_SIZE};
use qrfs_lib::bitmap::Bitmap;

#[derive(Parser, Debug)]
#[command(author, version, about)]
struct Args {
    /// Carpeta del sistema de archivos
    #[arg(value_name = "QR_FOLDER")]
    path: PathBuf,

    /// Nueva cantidad total de bloques
    #[arg(long)]
    new_size: u64,
}

fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    println!("{}", "=== QRFS Resizer ===".bold().blue());

    // 1. Setup
    let device = BlockDevice::new(&args.path)?;
    print!("Passphrase: ");
    std::io::stdout().flush()?;
    let password = read_password()?;

    // 2. Leer Superbloque
    let block0 = device.read_block(0)?;
    if block0.len() < 16 { anyhow::bail!("Disco corrupto"); }
    let (salt, encrypted_sb) = block0.split_at(16);
    let mut salt_arr = [0u8; 16];
    salt_arr.copy_from_slice(salt);

    let crypto = CryptoEngine::new(&password, salt_arr);
    let sb_bytes = crypto.decrypt(encrypted_sb).map_err(|_| anyhow::anyhow!("Contraseña incorrecta"))?;
    let mut sb: SuperBlock = bincode::deserialize(&sb_bytes)?;

    if sb.magic != QRFS_MAGIC {
        anyhow::bail!("No es un volumen QRFS válido");
    }

    println!("Tamaño actual: {} bloques", sb.total_blocks);
    println!("Tamaño deseado: {} bloques", args.new_size);

    if args.new_size == sb.total_blocks {
        println!("El tamaño es el mismo. Nada que hacer.");
        return Ok(());
    }

    // 3. Leer Bitmap
    let enc_bitmap = device.read_block(sb.bitmap_start)?;
    let bitmap_bytes = crypto.decrypt(&enc_bitmap)?;
    let mut bitmap: Bitmap = bincode::deserialize(&bitmap_bytes)?;

    // 4. Ejecutar Redimensión Lógica
    // Aquí usamos la función segura que agregamos al bitmap
    match bitmap.resize(args.new_size as usize) {
        Ok(_) => println!("{}", "[OK] Mapa de bits redimensionado en memoria.".green()),
        Err(e) => {
            println!("{}", format!("[ERROR] No se puede reducir: {}", e).red());
            return Ok(()); // Salimos sin guardar cambios
        }
    }

    // 5. Ejecutar Redimensión Física (Solo si reducimos)
    if args.new_size < sb.total_blocks {
        println!("Eliminando archivos físicos sobrantes...");
        device.trim(args.new_size, sb.total_blocks)?;
    }

    // 6. Actualizar Superbloque
    let old_blocks = sb.total_blocks;
    sb.total_blocks = args.new_size;
    
    // Recalcular bloques libres (Aproximación simple: sumar/restar diferencia)
    if args.new_size > old_blocks {
        sb.free_blocks_count += args.new_size - old_blocks;
    } else {
        sb.free_blocks_count -= old_blocks - args.new_size;
    }

    // 7. Guardar Cambios (Cifrar y Escribir)
    // A. Guardar Bitmap
    let new_bitmap_bytes = bincode::serialize(&bitmap)?;
    // Validación de seguridad: ¿Cabe el nuevo bitmap en su bloque?
    // Asumimos bloque único para bitmap por diseño actual del proyecto
    if new_bitmap_bytes.len() > (BLOCK_SIZE - 28) { 
        anyhow::bail!("El nuevo tamaño excede la capacidad del bloque de Bitmap. Límite alcanzado.");
    }
    let enc_new_bitmap = crypto.encrypt(&new_bitmap_bytes)?;
    device.write_block(sb.bitmap_start, &enc_new_bitmap)?;

    // B. Guardar Superbloque
    let new_sb_bytes = bincode::serialize(&sb)?;
    let enc_new_sb = crypto.encrypt(&new_sb_bytes)?;
    
    let mut new_block0 = Vec::new();
    new_block0.extend_from_slice(&crypto.salt);
    new_block0.extend_from_slice(&enc_new_sb);
    device.write_block(0, &new_block0)?;

    println!("{}", "¡Redimensión completada exitosamente!".bold().green());
    println!("Nuevo espacio libre: {} bloques", sb.free_blocks_count);

    Ok(())
}