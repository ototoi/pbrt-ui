use rust_embed::Embed;

#[derive(Embed)]
#[folder = "assets/"]
#[exclude = ".DS_Store"]
pub struct Assets;
