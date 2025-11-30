use clap::Parser;
use std::path::PathBuf;
use std::io::Write;
use rpassword::read_password;
use fuser::MountOption;
use qrfs_lib::device::BlockDevice;

mod fs; // Importamos el módulo fs.rs que acabamos de crear

#[derive(Parser, Debug)]
#[command(author, version, about)]
struct Args {
    /// Carpeta donde están los QRs (disco físico)
    #[arg(value_name = "QR_FOLDER")]
    source: PathBuf,

    /// Carpeta donde se montará el FS (disco lógico)
    #[arg(value_name = "MOUNT_POINT")]
    mountpoint: PathBuf,
}

fn main() -> anyhow::Result<()> {
    env_logger::init(); // Para ver logs con RUST_LOG=debug
    let args = Args::parse();

    // 1. Validar rutas
    if !args.source.exists() {
        anyhow::bail!("La carpeta de QRs no existe: {:?}", args.source);
    }
    if !args.mountpoint.exists() {
        std::fs::create_dir_all(&args.mountpoint)?;
    }

    // 2. Pedir contraseña
    print!("Password para montar QRFS: ");
    std::io::stdout().flush()?;
    let password = read_password()?;

    // 3. Inicializar Dispositivo
    let device = BlockDevice::new(&args.source)?;

    // 4. Intentar montar (Descifrar y cargar en RAM)
    println!("Descifrando sistema de archivos...");
    let filesystem = fs::QRFS::try_mount(device, &password)?;

    // 5. Iniciar FUSE
    println!("Montando en {:?}... (Ctrl+C para desmontar)", args.mountpoint);
    
    // Opciones de montaje estándar
    let options = vec![
        MountOption::RW,
        MountOption::FSName("qrfs".to_string()),
        MountOption::AutoUnmount, // Desmontar automáticamente al matar el proceso
    ];

    // Esta función bloquea el hilo hasta que se desmonte
    fuser::mount2(filesystem, &args.mountpoint, &options)?;

    Ok(())
}