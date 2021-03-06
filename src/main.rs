extern crate reqwest;
use cursive::align::HAlign;
use cursive::traits::*;
use cursive::Cursive;
use percent_encoding::{percent_encode, PATH_SEGMENT_ENCODE_SET};
use serde::{Deserialize, Serialize};
use std::sync::mpsc::{channel, Sender};
use std::thread;

use cursive::view::ScrollStrategy;
use cursive::views::{BoxView, Dialog, DummyView, EditView, LinearLayout, ScrollView, TextView};
use custom_error::custom_error;

custom_error! {ChatError
    JSONError{source: serde_json::error::Error} = @{
        source.to_string()
    },
    ReqwestError{source: reqwest::Error } = @{
        source.to_string().split(": ").collect::<Vec<&str>>()[1]
    },
    Unknown = "unknown error"
}

//These next structs are to deserialize the JSON responses into
#[derive(Deserialize)]
struct Response {
    t: Time,
    m: Vec<MessageResp>,
}

#[derive(Deserialize)]
struct MessageResp {
    d: Message,
}

#[derive(Deserialize)]
struct Time {
    t: String,
}

//Message is a sub object of MessageResp
#[derive(Serialize, Deserialize)]
struct Message {
    uuid: String,
    text: String,
}

fn main() {
    //We create two channels, one to pass the channel name to the subscribe function
    //Another to send new messages from the subscribe function to the UI
    let (channel_sender, channel_receiver) = channel();
    let (mut msg_sender, msg_receiver) = channel();

    //Create a seperate thread, this allows us to have a subscribe loop that wont stop the UI from updating
    let _handle1 = thread::spawn(move || {
        let mut time_token = "".to_string();
        println!("Subscribed to channel. Enter messages to publish!");
        //We wait for the UI to send us the channel name
        let test_channel = channel_receiver.recv();
        if test_channel.is_ok() {
            let channel_name: String = test_channel.unwrap();

            //Once we have the channel name, we create a loop that lets us request messages
            loop {
                let result: Result<String, ChatError> =
                    subscribe(&time_token, &mut msg_sender, &channel_name);
                if result.is_ok() {
                    //We update the time_token var to get all messages that happened after that specific time.
                    time_token = result.ok().unwrap();
                } else if result.is_err() {
                    let err = result.unwrap_err();
                    //If the request times out, thats okay, we just restart it with that same timetoken, looking for new messages.
                    if err.to_string() != "timed out" {
                        println!(
                            "Error: {:?} \nPlease restart application to try again.",
                            err.to_string()
                        );
                        break;
                    }
                }
            }
        }
    });

    // Creates the cursive root - required for every application.
    let mut siv = Cursive::default();
    //First layer - get username and channel
    siv.add_layer(
        Dialog::around(
            LinearLayout::vertical()
                .child(DummyView.fixed_height(1))
                .child(TextView::new("Enter Username").h_align(HAlign::Center))
                .child(EditView::new().with_id("username").fixed_width(20))
                .child(DummyView.fixed_height(1))
                .child(TextView::new("Enter Channel").h_align(HAlign::Center))
                .child(EditView::new().with_id("channel").fixed_width(20)),
        )
        .title("PubNub Chat")
        .button("Okay", move |s| {
            //Saving inputs content to variables to check them.
            let channel = s
                .call_on_id("channel", |view: &mut EditView| view.get_content())
                .unwrap();
            let username = s
                .call_on_id("username", |view: &mut EditView| view.get_content())
                .unwrap();
            //Checking if either input is empty.

            if username.is_empty() {
                s.add_layer(Dialog::info("Please enter a username !".to_string()));
            } else {
                let new_channel = if channel.is_empty() {
                    "global".to_string()
                } else {
                    channel.to_string()
                };

                channel_sender.send(new_channel).unwrap();
                s.pop_layer();
                s.add_layer(BoxView::with_fixed_size(
                    (40, 20),
                    Dialog::new()
                        .title("PubNub Chat")
                        .content(
                            //Instead of using a ListView, we use a ScrollView with a LinearLayout inside.
                            //This allows us to remove the extra lines from the View
                            LinearLayout::vertical()
                                .child(
                                    ScrollView::new(
                                        LinearLayout::vertical()
                                            .child(DummyView.fixed_height(1))
                                            //Add in a certain amount of dummy views, to make the new messages appear at the bottom
                                            .with(|messages| {
                                                for _ in 0..13 {
                                                    messages.add_child(DummyView.fixed_height(1));
                                                }
                                            })
                                            .child(DummyView.fixed_height(1))
                                            .with_id("messages"),
                                    )
                                    .scroll_strategy(ScrollStrategy::StickToBottom),
                                )
                                .child(EditView::new().with_id("message")),
                        )
                        .h_align(HAlign::Center)
                        .button("Send", move |s| {
                            let message = s
                                .call_on_id("message", |view: &mut EditView| view.get_content())
                                .unwrap();
                            let new_channel_2 = if channel.is_empty() {
                                "global".to_string()
                            } else {
                                channel.to_string()
                            };
                            if message.is_empty() {
                                s.add_layer(
                                    Dialog::new()
                                        .title("PubNub Chat")
                                        .content(TextView::new("Please enter a message!!"))
                                        .button("Okay", |s| {
                                            s.pop_layer();
                                        }),
                                )
                            } else {
                                let result = publish(
                                    message.to_string(),
                                    username.to_string(),
                                    new_channel_2,
                                );
                                if result.is_err() {
                                    //If there was an error then we say that there is one, and don't do anything.
                                    s.add_layer(
                                        Dialog::new()
                                            .title("PubNub Chat")
                                            .content(TextView::new("Error Publishing!"))
                                            .button("Okay", |s| {
                                                s.pop_layer();
                                            }),
                                    )
                                } else {
                                    //Clear out the EditView.
                                    s.call_on_id("message", |view: &mut EditView| {
                                        view.set_content("")
                                    })
                                    .unwrap();
                                }
                            }
                        })
                        .button("Quit", |s| s.quit()),
                ));
            }
        })
        .button("Quit", |s| s.quit())
        .h_align(HAlign::Center),
    );
    //This is where we check for updates from the subscribe function.
    //We have a message count  and a loop, refreshing whenever there is a new message coming in.
    let mut message_count = 0;
    siv.refresh();
    loop {
        siv.step();
        if !siv.is_running() {
            break;
        }

        let mut needs_refresh = false;
        //Non blocking channel receiver.
        for m in msg_receiver.try_iter() {
            siv.call_on_id("messages", |messages: &mut LinearLayout| {
                needs_refresh = true;
                message_count += 1;
                messages.add_child(TextView::new(m));
                if message_count <= 14 {
                    messages.remove_child(0);
                }
            });
        }
        if needs_refresh {
            siv.refresh();
        }
    }
}

fn subscribe(
    time: &str,
    msg_sender: &mut Sender<String>,
    channel: &str,
) -> Result<String, ChatError> {
    //Format the URL
    let channel = channel.to_string();
    let mut url = format!(
        "https://{host}/v2/subscribe/{subkey}/{channel}/0/",
        host = "ps.pndsn.com",
        subkey = "Enter_SUB_key",
        channel = percent_encode(channel.as_bytes(), PATH_SEGMENT_ENCODE_SET),
    );
    //If we provided a timetoken, then insert it.
    if !time.is_empty() {
        url.push_str(time);
    }
    //Get the reqwest response, then use our previously created scruts to deserialize it.
    let mut resp = reqwest::get(&url)?;
    if resp.status().is_success() {
        let deserialized: Response = serde_json::from_str(&resp.text()?).unwrap();
        let new_time = deserialized.t.t;
        for m in deserialized.m {
            //Send the new message to the UI above.
            msg_sender
                .send(format!("{} : {}", m.d.uuid, m.d.text))
                .unwrap();
        }
        return Ok(new_time);
    }
    Ok(time.to_string())
}

fn publish(text: String, uuid: String, channel: String) -> Result<(), ChatError> {
    let message = Message { uuid, text };
    let m_json = serde_json::to_string(&message)?;
    let url = format!(
        "https://{host}/publish/{pubkey}/{subkey}/0/{channel}/0/{message}",
        host = "ps.pndsn.com",
        pubkey = "Enter_PUB_Key",
        subkey = "Enter_SUB_Key",
        channel = percent_encode(channel.as_bytes(), PATH_SEGMENT_ENCODE_SET),
        message = percent_encode(m_json.as_bytes(), PATH_SEGMENT_ENCODE_SET),
    );
    let _resp = reqwest::get(&url)?;
    Ok(())
}
