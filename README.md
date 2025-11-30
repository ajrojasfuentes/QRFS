# QRFS
QRFS File System

**Instituto Tecnológico de Costa Rica**
**Curso:** Principios de Sistemas Operativos
**Proyecto 2:** Sistema de Archivos QRFS sobre Códigos QR
**Estudiantes:** Valery Carvajal, Anthony Rojas
**Fecha:** Noviembre, 2025

---

## 1. Introducción
El proyecto **QRFS (QR File System)** consiste en la implementación de un sistema de archivos en espacio de usuario (FUSE) que utiliza imágenes de códigos QR como medio de almacenamiento físico.

El objetivo principal es abstraer la complejidad de lectura y escritura de códigos QR para presentar al usuario una interfaz de sistema de archivos estándar (POSIX), donde pueda crear, leer y modificar archivos de manera transparente, mientras que internamente los datos son fragmentados, cifrados y almacenados en una matriz de imágenes `.png`.

Esta solución aborda retos de:
* **Abstracción de Hardware Virtual:** Tratar imágenes como bloques de disco.
* **Seguridad:** Implementación de cifrado AES-256 para proteger la estructura.
* **Gestión de Espacio:** Uso de mapas de bits y i-nodos personalizados.

## 2. Ambiente de Desarrollo
Para la implementación de este proyecto se utilizaron las siguientes herramientas y tecnologías:

* **Sistema Operativo:** GNU/Linux (Ubuntu/Debian).
* **Lenguaje de Programación:** Rust (Edición 2021).
* **Bibliotecas Principales:**
    * `fuser` (0.12): Binding de Rust para la interfaz FUSE del kernel.
    * `image` (0.24) y `qrcode` (0.12): Procesamiento de imágenes y generación de códigos.
    * `aes-gcm` y `pbkdf2`: Criptografía y derivación de claves.
    * `serde` / `bincode`: Serialización de estructuras en disco.
    * `printpdf` (0.5.3): Generación de reportes físicos.
* **Control de Versiones:** Git y GitHub.

## 3. Estructura de Datos y Funciones Principales

### 3.1 Estructuras de Datos (`types.rs`)
El diseño del sistema de archivos se basa en tres estructuras fundamentales que residen en el disco (imágenes QR):

1.  **SuperBloque (`SuperBlock`):**
    * Contiene la metadata global: número mágico, total de bloques, total de i-nodos y punteros al inicio del mapa de bits y la tabla de i-nodos.
    * Se almacena siempre en el **Bloque 0** junto con el *Salt* criptográfico.

2.  **Mapa de Bits (`Bitmap`):**
    * Estructura de bits donde `1` representa ocupado y `0` libre. Permite la asignación de bloques en tiempo constante $O(1)$ o lineal.

3.  **I-nodo (`Inode`):**
    * Representa un archivo o directorio. Contiene:
        * `mode`: Permisos y tipo.
        * `size`: Tamaño lógico en bytes.
        * `direct_blocks`: Arreglo de 12 punteros directos a bloques de datos (QRs).
    * Soporta archivos de hasta ~10KB (con configuración estándar) o más si se ajustan los punteros.

### 3.2 Mapeo Lógico-Físico
* **Unidad Lógica:** 1 Bloque = 1024 Bytes.
* **Unidad Física:** 1 Archivo PNG (`qr_XXXXX.png`) de aprox. $200 \times 200$ píxeles.
* **Traducción:** El módulo `device.rs` intercepta las peticiones de bloque, codifica los datos en Base64 para seguridad binaria, genera un código QR versión 40 y lo guarda como imagen.

### 3.3 Funciones FUSE Implementadas (`fs.rs`)
Se implementaron las siguientes llamadas al sistema:
* `getattr`: Recupera metadatos del i-nodo.
* `create` / `unlink`: Gestión del ciclo de vida de archivos.
* `read` / `write`: I/O de datos con cifrado transparente.
* `mkdir` / `rmdir`: Gestión básica de directorios.
* `rename`: Renombrado de archivos.
* `statfs`: Reporte de espacio libre (`df -h`).

## 4. Instrucciones de Ejecución

### Compilación
Desde la raíz del proyecto (donde está el `Cargo.toml` del workspace):
```bash
cargo build --release