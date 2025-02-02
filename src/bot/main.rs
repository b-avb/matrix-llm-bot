use matrix_sdk::{
    config::SyncSettings, room::Room, ruma::{events::room::{member::StrippedRoomMemberEvent, message::{MessageType, OriginalSyncRoomMessageEvent, RoomMessageEventContent}}, user_id
          }
};
use matrix_sdk::Client as MatrixClient;
use tokio::time::{sleep, Duration};
use dotenv::dotenv;
use std::fs;
use openai_api_rs::v1::chat_completion::{self, ChatCompletionRequest};
use openai_api_rs::v1::api::Client as OpenAIClient;

// Async function that awaits for an invitation and accepts it automatically
async fn handle_room_invitation(
    room_member: StrippedRoomMemberEvent,
    client: MatrixClient,
    room: Room,
) {
    if room_member.state_key != client.user_id().unwrap() {
        return;
    }

    if let Room::Invited(room) = room {
        tokio::spawn(async move {
            println!("Autojoining room {}", room.room_id());
            let mut delay = 2;

            while let Err(err) = room.accept_invitation().await {
                // retry autojoin due to synapse sending invites, before the
                // invited user can join for more information see
                // https://github.com/matrix-org/synapse/issues/4345
                eprintln!("Failed to join room {} ({err:?}), retrying in {delay}s", room.room_id());

                sleep(Duration::from_secs(delay)).await;
                delay *= 2;

                if delay > 3600 {
                    eprintln!("Can't join room {} ({err:?})", room.room_id());
                    break;
                }
            }
            println!("Successfully joined room {}", room.room_id());
        });
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenv().ok();

    let password = std::env::var("PASSWORD").expect("PASSWORD must be set.");

    let bot_user = user_id!("@virto_bot:matrix.org");
    let matrix_client = MatrixClient::builder().server_name(bot_user.server_name()).build().await?;
 
    // First we need to log in.
    matrix_client.login_username(bot_user, &password).send().await?;
    // We add an event handler that listens if our user is invited to a room
    // This event_handler should be different: it has to listen every time a guest joins and invite it into a new room where you can talk with the bot.
    matrix_client.add_event_handler(handle_room_invitation);
    // We add an event handler that listens if our user is invited to a room
    matrix_client.add_event_handler(move |ev: OriginalSyncRoomMessageEvent, room: Room| {
        
        async move {
            if let Err(e) = on_room_message(ev, room).await {
                eprintln!("Error processing message: {}", e);
            }
        }
    });
    
    //This event handler listens and prints every message it's received
    matrix_client.add_event_handler(|ev: OriginalSyncRoomMessageEvent| async move {
        if ev.sender != user_id!("@virto_bot:matrix.org") { 
            println!("Received a message {:?}", ev); 
        }   
     });

    // Syncing is important to synchronize the client state with the server.
    // This method will never return unless there is an error.
    matrix_client.sync(SyncSettings::default()).await?;

    Ok(())
}

// Async function that gets the text content of a room and answers. 
async fn on_room_message(event: OriginalSyncRoomMessageEvent, room: Room) -> Result<(), Box<dyn std::error::Error>> {
    let room = match room {
        Room::Joined(room) => room,
        _ => return Ok(()), // For now we ignore messages unrelated with rooms.
    };

    let text_content = match event.content.msgtype {
        MessageType::Text(text_content) => text_content, 
        _ => return Ok(()), // For now the bot only processes text messages.
    };

    // Ignore bot messages in order to avoid a infinite loop.
    let target_user_id = user_id!("@virto_bot:matrix.org");
    if event.sender == target_user_id {
        return Ok(());
    }

    // We check that the message we will process is the user message.
    let user_message = text_content.body.clone();
    println!("THis is the USER MESSAGE {}", user_message);

    let openai_key = std::env::var("OPENAI_API_KEY").expect("OPENAI_API_KEY must be set.");
    let openai_client = OpenAIClient::new(openai_key);

    //Processing file
    let file_content = fs::read_to_string("src/bot/virto.md").unwrap();

    let prompt = format!("Información del archivo: {}\nMensaje del usuario: {}", file_content, user_message);

    let req = ChatCompletionRequest::new(
        "gpt-4-0125-preview".into(),
        vec![chat_completion::ChatCompletionMessage {
            role: chat_completion::MessageRole::user,
            content: chat_completion::Content::Text(prompt),
            name: None,
        }],
    )
    .max_tokens(4096);

    println!("{:?}", req);
    let result: chat_completion::ChatCompletionResponse = openai_client.chat_completion(req)?;
    println!("Content: {:?}", result.choices[0].message.content);
    println!("Response Headers: {:?}", result.headers);
    
    let choice = result.choices.get(0).ok_or("No se ha encontrado un mensaje en este indice.")?;

    let Some(my_string) = choice.message.content.to_owned()
     else {
        return Err("El modelo no ha contestado.".into());
    };

    let content = RoomMessageEventContent::text_plain(my_string);
    room.send(content, None).await?;

    Ok(())
}
