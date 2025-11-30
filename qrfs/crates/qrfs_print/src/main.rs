use clap::Parser;
use std::path::PathBuf;
use std::fs::File;
use std::io::BufWriter;
use glob::glob;
use printpdf::*; // Importamos todo lo de printpd


#[derive(Parser, Debug)]
#[command(author, version, about)]
struct Args {
    #[arg(value_name = "QR_FOLDER")]
    path: PathBuf,

    #[arg(value_name = "OUTPUT_PDF", default_value = "backup_fs.pdf")]
    output: PathBuf,
}

fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    println!("Generando PDF simple desde: {:?}", args.path);

    // 1. Buscar archivos
    let pattern = args.path.join("qr_*.png");
    let mut files: Vec<PathBuf> = Vec::new();
    for entry in glob(pattern.to_str().unwrap())? {
        if let Ok(path) = entry {
            files.push(path);
        }
    }
    files.sort();

    if files.is_empty() {
        anyhow::bail!("No hay imágenes QR.");
    }

    // 2. Crear Documento (A4)
    let (doc, page1, layer1) = PdfDocument::new("QRFS", Mm(210.0), Mm(297.0), "Layer 1");
    let font = doc.add_builtin_font(BuiltinFont::Helvetica).unwrap();
    let mut current_layer = doc.get_page(page1).get_layer(layer1);

    // 3. Loop simple: 1 Imagen = 1 Página
    for (i, file_path) in files.iter().enumerate() {
        // Si no es la primera, nueva página
        if i > 0 {
            let (page, layer) = doc.add_page(Mm(210.0), Mm(297.0), "Layer 1");
            current_layer = doc.get_page(page).get_layer(layer);
        }

        let filename = file_path.file_name().unwrap().to_string_lossy();

        // A. Cargar imagen con la librería 'image'
        // Al usar image 0.24 en Cargo.toml, esto devuelve el tipo exacto que printpdf quiere.
        let img= image::open(file_path)?;

        // B. Convertir a objeto PDF
        // Ahora sí funcionará porque las versiones coinciden
        let image_file = Image::from_dynamic_image(&img);

        // C. Dibujar Título (Arriba)
        current_layer.use_text(format!("Archivo: {}", filename), 24.0, Mm(20.0), Mm(270.0), &font);

        // D. Dibujar Imagen (Centro)
        // Posición X=30mm, Y=100mm, Tamaño=150mm x 150mm
        image_file.add_to_layer(
            current_layer.clone(),
            Some(Mm(25.0)), Some(Mm(70.0)),
            None,
            Some(11.0), Some(11.0),
            None 
        );
        
        println!("Agregada página para: {}", filename);
    }

    // 4. Guardar
    let file = File::create(&args.output)?;
    let mut writer = BufWriter::new(file);
    doc.save(&mut writer)?;

    println!("¡PDF creado en {:?}!", args.output);
    Ok(())
}