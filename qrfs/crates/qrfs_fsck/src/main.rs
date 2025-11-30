use clap::Parser;
use std::path::PathBuf;
use std::io::Write;
use rpassword::read_password;
use colored::*; // Para output bonito
use std::collections::HashSet;

use qrfs_lib::device::BlockDevice;
use qrfs_lib::crypto::CryptoEngine;
use qrfs_lib::types::{SuperBlock, Inode, QRFS_MAGIC};
use qrfs_lib::bitmap::Bitmap;

#[derive(Parser, Debug)]
#[command(author, version, about)]
struct Args {
    /// Carpeta donde están los QRs
    #[arg(value_name = "QR_FOLDER")]
    path: PathBuf,
}

fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    println!("{}", "=== QRFS File System Check (fsck) ===".bold().blue());

    // 1. Validar acceso al dispositivo
    if !args.path.exists() {
        anyhow::bail!("La carpeta no existe");
    }
    let device = BlockDevice::new(&args.path)?;
    println!("[*] Dispositivo encontrado en {:?}", args.path);

    // 2. Autenticación
    print!("Passphrase: ");
    std::io::stdout().flush()?;
    let password = read_password()?;

    // 3. Leer Bloque 0 (Superbloque)
    println!("[*] Leyendo Superbloque...");
    let block0 = device.read_block(0)?;
    if block0.len() < 16 {
        println!("{}", "[FAIL] Bloque 0 corrupto o ilegible".red());
        return Ok(());
    }
    let (salt, encrypted_sb) = block0.split_at(16);
    let mut salt_arr = [0u8; 16];
    salt_arr.copy_from_slice(salt);

    let crypto = CryptoEngine::new(&password, salt_arr);
    
    // Intentar descifrar
    let sb_bytes = match crypto.decrypt(encrypted_sb) {
        Ok(b) => b,
        Err(_) => {
            println!("{}", "[FAIL] No se pudo descifrar el Superbloque. ¿Contraseña incorrecta?".red());
            return Ok(());
        }
    };

    let sb: SuperBlock = bincode::deserialize(&sb_bytes)?;
    
    // Verificar Magic Number
    if sb.magic == QRFS_MAGIC {
        println!("{}", "[OK] Firma QRFS válida (Magic Number correcto)".green());
    } else {
        println!("{}", "[FAIL] Firma inválida. No es un sistema QRFS".red());
        return Ok(());
    }

    println!("    > Total Blocks: {}", sb.total_blocks);
    println!("    > Inodes: {}", sb.total_inodes);

    // 4. Leer y Verificar Bitmap
    println!("[*] Verificando Mapa de Bits...");
    let enc_bitmap = device.read_block(sb.bitmap_start)?;
    let bitmap_bytes = crypto.decrypt(&enc_bitmap)?;
    let stored_bitmap: Bitmap = bincode::deserialize(&bitmap_bytes)?;
    println!("{}", "[OK] Bitmap descifrado y legible".green());

    // 5. Analizar Inodos y Recalcular Bitmap Real
    println!("[*] Analizando Tabla de Inodos...");
    let enc_inodes = device.read_block(sb.inode_table_start)?;
    let inodes_bytes = crypto.decrypt(&enc_inodes)?;
    let inode_list: Vec<Inode> = bincode::deserialize(&inodes_bytes)?;

    // Vamos a reconstruir qué bloques están REALMENTE en uso
    let mut calculated_used_blocks = HashSet::new();
    
    // Agregamos bloques de metadatos que sabemos que existen
    calculated_used_blocks.insert(0); // Superbloque
    calculated_used_blocks.insert(sb.bitmap_start); // Bitmap
    calculated_used_blocks.insert(sb.inode_table_start); // Tabla inodos (simplificado a 1 bloque)

    let mut valid_inodes_count = 0;

    for (idx, inode) in inode_list.iter().enumerate() {
        // Si el inodo tiene modo 0, está "borrado" o vacío
        if inode.mode != 0 {
            valid_inodes_count += 1;
            
            // Revisar sus bloques de datos
            for &block_id in inode.direct_blocks.iter() {
                if block_id != 0 {
                    if block_id >= sb.total_blocks {
                        println!("    {} Inodo {} apunta a bloque fuera de rango: {}", "[ERROR]".red(), idx, block_id);
                    } else {
                        calculated_used_blocks.insert(block_id);
                    }
                }
            }
        }
    }

    println!("    > Inodos activos encontrados: {}", valid_inodes_count);

    // 6. Comparación Final (Stored vs Calculated)
    println!("[*] Buscando inconsistencias...");
    let mut errors = 0;

    // Chequear Falsos Libres (El bitmap dice libre, pero un inodo lo usa) -> GRAVE
    for &block_id in &calculated_used_blocks {
        if !stored_bitmap.get(block_id as usize) {
            println!("    {} Bloque {} está en uso por un archivo pero marcado como LIBRE en bitmap", "[CORRUPCIÓN]".red(), block_id);
            errors += 1;
        }
    }

    // Chequear Falsos Ocupados (El bitmap dice ocupado, pero nadie lo usa) -> LEAK (Huérfano)
    // Recorremos todo el bitmap
    for i in 0..sb.total_blocks {
        if stored_bitmap.get(i as usize) {
            if !calculated_used_blocks.contains(&i) {
                println!("    {} Bloque {} marcado como ocupado pero nadie lo usa (Huérfano)", "[WARN]".yellow(), i);
                // Aquí podríamos ofrecer repararlo (fsck -r) poniendo el bit en 0
            }
        }
    }

    if errors == 0 {
        println!("\n{}", ">> EL SISTEMA DE ARCHIVOS ESTÁ SANO".bold().green());
    } else {
        println!("\n{} Se encontraron {} errores graves.", ">> PRECAUCIÓN:".bold().red(), errors);
    }

    Ok(())
}