//! The D-Bus side: owns the MPRIS name and object, and applies updates to it.

use futures::channel::mpsc::UnboundedReceiver;
use futures::StreamExt;
use zbus::Connection;

use crate::mpris::{RiffMpris, RiffMprisPlayer};
use crate::updates::MprisUpdate;

#[tokio::main]
pub async fn run(
    mpris: RiffMpris,
    player: RiffMprisPlayer,
    receiver: UnboundedReceiver<MprisUpdate>,
) -> zbus::Result<()> {
    let connection = Connection::session().await?;
    connection
        .object_server()
        .at("/org/mpris/MediaPlayer2", mpris)
        .await?;
    connection
        .object_server()
        .at("/org/mpris/MediaPlayer2", player)
        .await?;
    connection
        .request_name("org.mpris.MediaPlayer2.dev.diegovsky.Riff")
        .await?;

    let player_ref = connection
        .object_server()
        .interface::<_, RiffMprisPlayer>("/org/mpris/MediaPlayer2")
        .await?;
    receiver
        .for_each(|update| async {
            let mut player = player_ref.get_mut().await;
            if let Err(e) = player.apply(update, player_ref.signal_emitter()).await {
                log::warn!("Could not tell the desktop about a playback change: {e}");
            }
        })
        .await;

    Ok(())
}
