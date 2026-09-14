//! Export a rendered frame of the application with its normal asset and scene inputs.

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    use bevy::{
        app::AppExit,
        prelude::*,
        render::view::screenshot::{save_to_disk, Screenshot, ScreenshotCaptured},
    };

    let mut args = std::env::args().skip(1);
    let path = args
        .next()
        .expect("usage: capture <output.png> [delay-seconds]");
    let delay: f64 = args
        .next()
        .map(|value| value.parse().expect("delay must be seconds"))
        .unwrap_or(20.0);
    let source = moly_app::asset_source::resolve().expect("asset source");
    let site = moly_app::site_request::resolve().expect("site request");
    let mut app = moly_game::app(source, site);
    app.add_systems(
        Update,
        move |mut commands: Commands, time: Res<Time<Real>>, mut requested: Local<bool>| {
            if !*requested && time.elapsed_secs_f64() >= delay {
                *requested = true;
                let mut save = save_to_disk(path.clone());
                commands.spawn(Screenshot::primary_window()).observe(
                    move |event: On<ScreenshotCaptured>, mut exit: MessageWriter<AppExit>| {
                        save(event);
                        exit.write(AppExit::Success);
                    },
                );
            }
        },
    );
    app.run();
}

#[cfg(target_arch = "wasm32")]
fn main() {}
