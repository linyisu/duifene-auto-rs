use std::env;
use std::fs;
use std::path::{Path, PathBuf};

const ICON_COLOR: &str = "#1c1917";
const ICON_SIZES: &[u32] = &[16, 32, 48, 64, 128, 256];

fn main() {
    println!("cargo:rerun-if-changed=assets/pen.svg");

    let output_directory = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR is required"));
    let svg_path = Path::new("assets/pen.svg");
    let svg = fs::read_to_string(svg_path).expect("failed to read assets/pen.svg");
    let svg = svg.replace("currentColor", ICON_COLOR);

    let tree = {
        let options = resvg::usvg::Options::default();
        resvg::usvg::Tree::from_str(&svg, &options).expect("failed to parse pen.svg")
    };

    let png_path = output_directory.join("icon.png");
    render_png(&tree, 256, &png_path);

    let mut icon_directory = ico::IconDir::new(ico::ResourceType::Icon);
    for size in ICON_SIZES {
        let pixmap = render_pixmap(&tree, *size);
        let image = ico::IconImage::from_rgba_data(*size, *size, pixmap.data().to_vec());
        icon_directory
            .add_entry(ico::IconDirEntry::encode(&image).expect("failed to encode icon image"));
    }
    let ico_path = output_directory.join("icon.ico");
    let ico_file = fs::File::create(&ico_path).expect("failed to create icon.ico");
    icon_directory
        .write(ico_file)
        .expect("failed to write icon.ico");

    println!(
        "cargo:rustc-env=DUIFENE_AUTO_ICON_PNG={}",
        png_path.display()
    );

    if env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let resource_path = output_directory.join("duifene-auto.rc");
        let resource_icon_path = ico_path.to_string_lossy().replace('\\', "/");
        fs::write(
            &resource_path,
            format!("1 ICON \"{}\"\n", resource_icon_path),
        )
        .expect("failed to write Windows resource file");
        embed_resource::compile(&resource_path, embed_resource::NONE)
            .manifest_optional()
            .expect("failed to embed Windows application icon");
    }
}

fn render_pixmap(tree: &resvg::usvg::Tree, size: u32) -> tiny_skia::Pixmap {
    let mut pixmap = tiny_skia::Pixmap::new(size, size).expect("failed to allocate icon pixmap");
    let source_size = tree.size();
    let scale = (size as f32 / source_size.width()).min(size as f32 / source_size.height());
    let transform = tiny_skia::Transform::from_scale(scale, scale);
    resvg::render(tree, transform, &mut pixmap.as_mut());
    pixmap
}

fn render_png(tree: &resvg::usvg::Tree, size: u32, path: &Path) {
    let pixmap = render_pixmap(tree, size);
    pixmap.save_png(path).expect("failed to write icon.png");
}
