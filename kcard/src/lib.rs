use kinode_process_lib::{
    call_init, eth, http,
    kernel_types::{KernelCommand, KernelPrint, KernelPrintResponse, KernelResponse},
    net, println, Address, LazyLoadBlob, Message, Request,
};

const BACKGROUND: &[u8] = include_bytes!("redsunset.jpeg");

const BEAUTIFUL_BIRD: &str = r#"
.`
`@@,,                     ,*
`@%@@@,            ,~-##`
~@@#@%#@@,      #####
  ~-%######@@@, #####
     -%%#######@#####,
       ~^^%##########@
          >^#########@
            `>#######`
           .>######%
          /###%^#%
        /##%@#  `
     ./######`
   /.^`.#^#^`
  `   ,#`#`#,
     ,/ /` `
   .*`
"#;

/// From kns_indexer process
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
struct KnsState {
    chain_id: u64,
    // what contract this state pertains to
    contract_address: eth::Address,
    // namehash to human readable name
    names: std::collections::HashMap<String, String>,
    // human readable name to most recent on-chain routing information as json
    // TODO: optional params knsUpdate? also include tba.
    nodes: std::collections::HashMap<String, net::KnsUpdate>,
    // last block we have an update from
    last_block: u64,
}

wit_bindgen::generate!({
    path: "target/wit",
    world: "kcard-mothu-et-doria-dot-os-v0",
    generate_unused_types: true,
    additional_derives: [serde::Deserialize, serde::Serialize],
});

call_init!(init);
fn init(our: Address) {
    let mut server = http::server::HttpServer::new(5);
    loop {
        match fetch_data(&our) {
            Ok(text) => {
                server
                    .bind_http_path(
                        "/kcard.png",
                        http::server::HttpBindingConfig::new(
                            false,
                            false,
                            false,
                            Some(LazyLoadBlob::new(
                                Some("image/png"),
                                write_text(our.node(), &text),
                            )),
                        ),
                    )
                    .expect("error binding http");
            }
            Err(e) => {
                println!("error fetching card data: {e:?}");
            }
        }

        // sleep 10 minutes then re-render
        std::thread::sleep(std::time::Duration::from_secs(600));
    }
}

fn write_text(our: &str, text: &str) -> Vec<u8> {
    // red sunset from https://www.metmuseum.org/art/collection/search/436833
    // public domain image
    let mut image = image::load_from_memory(BACKGROUND).expect("error loading background");
    let img_height = image.height() as i32;
    let img_width = image.width() as i32;
    // open source font Ubuntu Mono
    let font = ab_glyph::FontRef::try_from_slice(include_bytes!("UbuntuMono-Regular.ttf")).unwrap();

    let main_font_height = 17.0;
    let scale = ab_glyph::PxScale {
        x: main_font_height,
        y: main_font_height,
    };

    let font_color = image::Rgba([255u8, 255u8, 255u8, 255u8]);

    // draw our name

    imageproc::drawing::draw_text_mut(
        &mut image,
        image::Rgba([255u8, 255u8, 255u8, 255u8]),
        32,
        64,
        ab_glyph::PxScale { x: 36.0, y: 36.0 },
        &font,
        our,
    );

    // draw the beautiful bird

    let x_offset = 750;
    let mut y_offset = 32;

    for line in BEAUTIFUL_BIRD.lines() {
        imageproc::drawing::draw_text_mut(
            &mut image, font_color, x_offset, y_offset, scale, &font, line,
        );
        y_offset += main_font_height as i32;
    }

    // draw the system info

    let x_offset = 32;
    let mut y_offset = 128;

    for line in text.lines() {
        imageproc::drawing::draw_text_mut(
            &mut image, font_color, x_offset, y_offset, scale, &font, line,
        );
        y_offset += main_font_height as i32;
    }

    // write the current date and time

    imageproc::drawing::draw_text_mut(
        &mut image,
        font_color,
        x_offset,
        img_height - 36,
        ab_glyph::PxScale { x: 12.0, y: 12.0 },
        &font,
        &format!(
            "kcard rendered {}",
            chrono::DateTime::<chrono::Local>::from(std::time::SystemTime::now())
                .format("%m-%d-%Y %H:%M:%S")
                .to_string(),
        ),
    );

    // write attribution for image

    imageproc::drawing::draw_text_mut(
        &mut image,
        font_color,
        img_width - 196,
        img_height - 36,
        ab_glyph::PxScale { x: 12.0, y: 12.0 },
        &font,
        "Red Sunset // Arkhyp Kuindzhi",
    );

    // fin

    let mut buf = std::io::Cursor::new(vec![0; image.width() as usize * image.height() as usize]);
    image
        .write_to(&mut buf, image::ImageFormat::Png)
        .expect("error writing image");

    buf.into_inner()
}

fn fetch_data(our: &Address) -> anyhow::Result<String> {
    // get identity
    let Ok(Ok(Message::Response { body, .. })) = Request::to(("our", "net", "distro", "sys"))
        .body(rmp_serde::to_vec(&net::NetAction::GetPeer(our.node.clone())).unwrap())
        .send_and_await_response(60)
    else {
        return Err(anyhow::anyhow!("failed to get response from net (GetPeer)"));
    };
    let Ok(net::NetResponse::Peer(Some(our_id))) = rmp_serde::from_slice(&body) else {
        return Err(anyhow::anyhow!("got malformed response from net (GetPeer)"));
    };

    // get actively connected peers
    let Ok(Message::Response { body, .. }) = Request::new()
        .target(("our", "net", "distro", "sys"))
        .body(rmp_serde::to_vec(&net::NetAction::GetPeers).unwrap())
        .send_and_await_response(60)
        .unwrap()
    else {
        return Err(anyhow::anyhow!(
            "failed to get response from net (GetPeers)"
        ));
    };
    let Ok(net::NetResponse::Peers(peers)) = rmp_serde::from_slice(&body) else {
        return Err(anyhow::anyhow!(
            "got malformed response from net (GetPeers)"
        ));
    };
    let connected_peers = peers.into_iter().map(|p| p.name).collect::<Vec<String>>();

    // get eth providers
    let Ok(Message::Response { body, .. }) = Request::new()
        .target(("our", "eth", "distro", "sys"))
        .body(serde_json::to_vec(&eth::EthConfigAction::GetProviders).unwrap())
        .send_and_await_response(60)
        .unwrap()
    else {
        return Err(anyhow::anyhow!(
            "failed to get response from eth (GetProviders)"
        ));
    };
    let Ok(eth::EthConfigResponse::Providers(providers)) = serde_json::from_slice(&body) else {
        return Err(anyhow::anyhow!(
            "failed to parse eth response (GetProviders)"
        ));
    };

    // get eth subs
    let Ok(Message::Response { body, .. }) = Request::new()
        .target(("our", "eth", "distro", "sys"))
        .body(serde_json::to_vec(&eth::EthConfigAction::GetState).unwrap())
        .send_and_await_response(60)
        .unwrap()
    else {
        return Err(anyhow::anyhow!(
            "failed to get response from eth (GetState)"
        ));
    };
    let Ok(eth::EthConfigResponse::State {
        active_subscriptions,
        ..
    }) = serde_json::from_slice(&body)
    else {
        return Err(anyhow::anyhow!("failed to parse eth response (GetState)"));
    };

    // get number of processes
    let Ok(Message::Response { body, .. }) = Request::new()
        .target(("our", "kernel", "distro", "sys"))
        .body(serde_json::to_vec(&KernelCommand::Debug(KernelPrint::ProcessMap)).unwrap())
        .send_and_await_response(60)
        .unwrap()
    else {
        return Err(anyhow::anyhow!(
            "failed to get response from kernel (Debug(ProcessMap))"
        ));
    };
    let Ok(KernelResponse::Debug(KernelPrintResponse::ProcessMap(map))) =
        serde_json::from_slice::<KernelResponse>(&body)
    else {
        return Err(anyhow::anyhow!(
            "failed to parse kernel response (Debug(ProcessMap))"
        ));
    };
    let num_processes = map.len();
    Ok(make_text(
        our_id,
        connected_peers,
        providers,
        // sum up all the subscriptions
        active_subscriptions
            .values()
            .map(|v| v.len())
            .sum::<usize>(),
        num_processes,
        fetch_kns_state()?,
    ))
}

fn make_text(
    our_id: net::Identity,
    connected_peers: Vec<String>,
    providers: std::collections::HashSet<eth::ProviderConfig>,
    active_subscriptions: usize,
    num_processes: usize,
    kns_state: KnsState,
) -> String {
    let mut providers = providers
        .into_iter()
        .map(|p| p.chain_id)
        // remove duplicates
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    providers.sort();
    let chain_ids = providers
        .iter()
        .map(|p| p.to_string())
        .collect::<Vec<_>>()
        .join(",\n                 ");
    format!(
        r#"
...is running {} processes
...using public key
   {}
...with {}
...with {} eth providers
   for chain IDs {}
...and has {} active eth subscriptions.

connected to {} peers out of {} known
from kimap {}
"#,
        num_processes,
        our_id.networking_key,
        routing_to_string(our_id.routing),
        providers.len(),
        chain_ids,
        active_subscriptions,
        connected_peers.len(),
        kns_state.nodes.len(),
        kns_state.contract_address,
    )
}

fn routing_to_string(routing: net::NodeRouting) -> String {
    match routing {
        net::NodeRouting::Direct { ip: _, ports } => format!(
            "direct routing on {}",
            ports.into_keys().into_iter().collect::<Vec<_>>().join(", ")
        ),
        net::NodeRouting::Routers(routers) => {
            format!("indirect routing using {} routers", routers.len())
        }
    }
}

fn fetch_kns_state() -> anyhow::Result<KnsState> {
    let Ok(Message::Response { body, .. }) =
        Request::to(("our", "kns_indexer", "kns_indexer", "sys"))
            .body(
                serde_json::json!({
                    "GetState": {
                        "block": 0
                    }
                })
                .to_string()
                .as_bytes()
                .to_vec(),
            )
            .send_and_await_response(60)
            .unwrap()
    else {
        return Err(anyhow::anyhow!(
            "failed to get response from kns_indexer (GetState)"
        ));
    };
    let Ok(state) = serde_json::from_slice::<KnsState>(&body) else {
        return Err(anyhow::anyhow!(
            "failed to parse kns_indexer response (GetState)"
        ));
    };
    Ok(state)
}
