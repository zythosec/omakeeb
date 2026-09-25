//! omakeeb: configure a VIA keyboard on Omarchy, without a browser.

mod app;
mod board;
mod ui;

use std::path::PathBuf;
use std::process::ExitCode;

use app::{Launch, Omakeeb};
use gpui_kit::{
    AppContext as _, Bounds, TitlebarOptions, WindowBounds, WindowOptions, point, px, size,
};

const USAGE: &str = "\
omakeeb — configure VIA keyboards on Omarchy

usage: omakeeb [OPTIONS]

With no options, omakeeb looks for a VIA keyboard. The first time a keyboard
does not describe its own layout, choose the manufacturer's VIA JSON file.
That file is saved for the keyboard's USB ids and reused on the next plug-in.

options:
      --definition FILE   use this VIA definition, and save it for the
                          keyboard that is plugged in
      --demo              open a sample 60% keyboard, with no device
      --rules             print how to let this user open raw HID
  -h, --help              show this help
";

// The component icons embed a fixed set. Icons omakeeb draws itself are
// listed here, so only these SVGs are added to the binary.
gpui_kit::assets::icon_assets!(ExtraIcons, [Pencil]);

/// omakeeb's mark, drawn beside the title in the theme's colour.
pub const MARK: &str = "brand/omakeeb-mark.svg";

/// The component icons, then omakeeb's own.
#[derive(Debug)]
struct AppAssets;

impl gpui_kit::AssetSource for AppAssets {
    fn load(&self, path: &str) -> gpui_kit::Result<Option<std::borrow::Cow<'static, [u8]>>> {
        if path == MARK {
            return Ok(Some(std::borrow::Cow::Borrowed(include_bytes!(
                "../../../assets/omakeeb-mark.svg"
            ))));
        }
        if let Some(bytes) = ExtraIcons.load(path)? {
            return Ok(Some(bytes));
        }
        gpui_kit::assets::Assets.load(path)
    }

    fn list(&self, path: &str) -> gpui_kit::Result<Vec<gpui_kit::SharedString>> {
        let mut paths = gpui_kit::assets::Assets.list(path)?;
        paths.extend(ExtraIcons.list(path)?);
        if MARK.starts_with(path) {
            paths.push(MARK.into());
        }
        paths.sort();
        paths.dedup();
        Ok(paths)
    }
}

fn main() -> ExitCode {
    let launch = match parse_args() {
        Ok(launch) => launch,
        Err(message) => {
            eprintln!("{message}");
            return ExitCode::from(2);
        }
    };
    if launch.rules {
        println!("{}", app::UDEV_HELP);
        return ExitCode::SUCCESS;
    }

    let launch_for_window = launch.clone();
    gpui_kit::application()
        .with_assets(AppAssets)
        .run(move |cx| {
            gpui_omarchy::init(cx);
            let launch = launch_for_window.clone();
            let window = cx
                .open_window(
                    WindowOptions {
                        window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                            point(px(120.), px(80.)),
                            size(px(1280.), px(800.)),
                        ))),
                        titlebar: Some(TitlebarOptions {
                            title: Some("omakeeb".into()),
                            ..Default::default()
                        }),
                        // The Wayland class. The desktop entry's StartupWMClass, window
                        // rules and the bar button's focus-if-open all match on it.
                        app_id: Some("omakeeb".into()),
                        // Only a floor for floating windows; tiling ignores it. The layout
                        // stacks and scrolls to fit, so this just keeps the bar usable.
                        window_min_size: Some(size(px(360.), px(280.))),
                        ..Default::default()
                    },
                    move |_, cx| cx.new(|cx| Omakeeb::new(launch.clone(), cx)),
                )
                .expect("open the omakeeb window");
            let _ = window.update(cx, |this, window, cx| {
                window.focus(&this.focus, cx);
            });
            cx.activate(true);
        });
    ExitCode::SUCCESS
}

fn parse_args() -> Result<Launch, String> {
    let mut demo = false;
    let mut rules = false;
    let mut definition = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => {
                println!("{USAGE}");
                std::process::exit(0);
            }
            "--demo" => demo = true,
            "--rules" => rules = true,
            "--definition" => {
                let path = args
                    .next()
                    .ok_or_else(|| "--definition needs a JSON file".to_owned())?;
                definition = Some(PathBuf::from(path));
            }
            other if other.starts_with('-') => {
                return Err(format!("unknown option {other}\n\n{USAGE}"));
            }
            other => return Err(format!("unexpected argument {other}\n\n{USAGE}")),
        }
    }
    if demo && definition.is_some() {
        return Err("--demo and --definition cannot be combined".to_owned());
    }
    if let Some(path) = &definition {
        let text = std::fs::read_to_string(path)
            .map_err(|err| format!("cannot read {}: {err}", path.display()))?;
        omakeeb_core::Definition::parse(&text)
            .map_err(|err| format!("{}: {err}", path.display()))?;
    }
    Ok(Launch {
        demo,
        rules,
        definition,
    })
}
