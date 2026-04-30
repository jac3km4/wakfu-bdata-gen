# Wakfu Binary Data Generator (`wakfu-bdata-gen`)

`wakfu-bdata-gen` is a Rust tool designed to extract binary data structures from the `wakfu-client.jar` file and automatically generate corresponding Rust structures for decoding and analyzing Wakfu game data.

## Features

* **Bytecode Analysis**: Utilizes `noak` to parse and analyze the JVM bytecode of the Wakfu client JAR.
* **Deobfuscation & Layout Extraction**: Automatically finds the binary data interface and extracts the fields, types, and ordinals of obfuscated structures.
* **Historical Mapping**: Compares extracted structures against a known set of original definitions (stored as RON files in `assets/structures`) to map obfuscated names back to their original names and structure definitions.
* **Code Generation**: Generates strongly-typed Rust structures complete with `serde` serialization and a custom `Decode` implementation for reading Wakfu's binary data formats.

## How It Works

1. **Class Loading**: Reads all `.class` files from `lib/wakfu-client.jar` inside the game's root directory.
2. **Interface Discovery**: Searches for specific interface implementations to identify binary data structures.
3. **Structure Parsing**: Parses the fields of the identified classes, mapping Java types (like `java/lang/Integer`, arrays, etc.) to corresponding Rust types (`i32`, `Vec`, etc.).
4. **Enum Ordinals Extraction**: Parses the `<clinit>` method of the binary data enum to extract the ordinals for each structure type.
5. **Rust Export**: Outputs standard Rust `.rs` files containing the generated structs and their properties.

## Usage

To run the generator, you need to provide the path to your Wakfu game root folder (which must contain the `lib/wakfu-client.jar` file) and the output directory for the generated Rust files.

```sh
cargo run --release -- <game_root_path> <output_directory>
```

### Example
```sh
cargo run --release -- "C:\Program Files\Wakfu" "./generated_bdata"
```

## Output Structure

The generated output directory will contain:
- A `mod.rs` file exporting all the extracted structures.
- One `.rs` file per extracted data structure (e.g., `achievement.rs`, `monster.rs`), containing the Rust `struct` definition and a `Decode` trait implementation for reading from a binary stream.

## Requirements
* Rust 2024 Edition (or latest stable)
* The original `wakfu-client.jar` (accessible within the game root directory).

## License

This project is licensed under the standard Rust project terms.
