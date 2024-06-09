use kinode_process_lib::{
    await_message, call_init, eth, http,
    kernel_types::{KernelCommand, KernelPrint, KernelPrintResponse, KernelResponse},
    net, println, Address, Message, Request,
};

const BACKGROUND: &[u8] = include_bytes!("redsunset.jpeg");

wit_bindgen::generate!({
    path: "target/wit",
    world: "kcard-ochocinco-dot-os-v0",
    generate_unused_types: true,
    additional_derives: [serde::Deserialize, serde::Serialize],
});

fn handle_message(_our: &Address) -> anyhow::Result<()> {
    let _message = await_message()?;
    Ok(())
}

call_init!(init);
fn init(our: Address) {
    println!("begin");

    // get identity
    let Ok(Ok(Message::Response { body, .. })) = Request::to(("our", "net", "distro", "sys"))
        .body(rmp_serde::to_vec(&net::NetAction::GetPeer(our.node.clone())).unwrap())
        .send_and_await_response(60)
    else {
        println!("failed to get response from net");
        return;
    };
    let Ok(net::NetResponse::Peer(Some(our_id))) = rmp_serde::from_slice(&body) else {
        println!("got malformed response from net");
        return;
    };

    // get eth providers
    let Ok(Message::Response { body, .. }) = Request::new()
        .target(("our", "eth", "distro", "sys"))
        .body(serde_json::to_vec(&eth::EthConfigAction::GetProviders).unwrap())
        .send_and_await_response(60)
        .unwrap()
    else {
        println!("failed to get response from eth");
        return;
    };
    let Ok(eth::EthConfigResponse::Providers(providers)) = serde_json::from_slice(&body) else {
        println!("failed to parse eth response");
        return;
    };

    // get eth subs
    let Ok(Message::Response { body, .. }) = Request::new()
        .target(("our", "eth", "distro", "sys"))
        .body(serde_json::to_vec(&eth::EthConfigAction::GetState).unwrap())
        .send_and_await_response(60)
        .unwrap()
    else {
        println!("failed to get response from eth");
        return;
    };
    let Ok(eth::EthConfigResponse::State {
        active_subscriptions,
        outstanding_requests,
    }) = serde_json::from_slice(&body)
    else {
        println!("failed to parse eth response");
        return;
    };

    // get number of processes
    let Ok(Message::Response { body, .. }) = Request::new()
        .target(("our", "kernel", "distro", "sys"))
        .body(serde_json::to_vec(&KernelCommand::Debug(KernelPrint::ProcessMap)).unwrap())
        .send_and_await_response(60)
        .unwrap()
    else {
        println!("failed to get response from kernel");
        return;
    };
    let Ok(KernelResponse::Debug(KernelPrintResponse::ProcessMap(map))) =
        serde_json::from_slice::<KernelResponse>(&body)
    else {
        println!("failed to parse kernel response");
        return;
    };
    let num_processes = map.len();
    let text = print_bird(
        &our,
        our_id,
        providers,
        // sum up all the subscriptions
        active_subscriptions
            .values()
            .map(|v| v.len())
            .sum::<usize>(),
        outstanding_requests.len() as usize,
        num_processes,
    );

    http::bind_http_static_path(
        "/kcard.png",
        false,
        false,
        Some("image/png".to_string()),
        write_text(&text.lines().collect::<Vec<&str>>()),
    )
    .expect("error binding http");

    loop {
        if let Err(e) = handle_message(&our) {
            println!("error: {e:?}");
        }
    }
}

fn write_text(text: &[&str]) -> Vec<u8> {
    // red sunset from https://www.metmuseum.org/art/collection/search/436833
    // public domain image
    let mut image = image::load_from_memory(BACKGROUND).expect("error loading background");
    // open source font Ubuntu Mono
    let font = ab_glyph::FontRef::try_from_slice(include_bytes!("UbuntuMono-Regular.ttf")).unwrap();

    let font_height = 48.0;
    let scale = ab_glyph::PxScale {
        x: font_height,
        y: font_height,
    };

    let mut y_offset = image.height() as i32 / 4;
    let x_offset = 50;

    for line in text {
        imageproc::drawing::draw_text_mut(
            &mut image,
            image::Rgba([255u8, 255u8, 255u8, 255u8]),
            x_offset,
            y_offset,
            scale,
            &font,
            line,
        );
        y_offset += font_height as i32;
    }

    let mut buf = std::io::Cursor::new(vec![0; image.width() as usize * image.height() as usize]);
    image
        .write_to(&mut buf, image::ImageFormat::Png)
        .expect("error writing image");

    buf.into_inner()
}

fn print_bird(
    our: &Address,
    our_id: net::Identity,
    providers: std::collections::HashSet<eth::ProviderConfig>,
    active_subscriptions: usize,
    outstanding_requests: usize,
    num_processes: usize,
) -> String {
    format!(
        r#"
    .`
`@@,,                     ,*   {}
  `@%@@@,            ,~-##`
    ~@@#@%#@@,      #####
      ~-%######@@@, #####
         -%%#######@#####,     pubkey: {}
           ~^^%##########@     routing: {}
              >^#########@
                `>#######`     {} eth providers for chain IDs {}
               .>######%       {} active eth subscriptions
              /###%^#%         {} outstanding eth requests
            /##%@#  `
         ./######`
       /.^`.#^#^`
      `   ,#`#`#,              {} running processes
         ,/ /` `
       .*`
                   "#,
        our.node(),
        our_id.networking_key,
        routing_to_string(our_id.routing),
        providers.len(),
        providers
            .into_iter()
            .map(|p| p.chain_id.to_string())
            // remove duplicates
            .collect::<std::collections::HashSet<_>>()
            .into_iter()
            .collect::<Vec<_>>()
            .join(", "),
        active_subscriptions,
        outstanding_requests,
        num_processes
    )
}

fn routing_to_string(routing: net::NodeRouting) -> String {
    match routing {
        net::NodeRouting::Direct { ip, ports } => format!(
            "direct at {} with {}",
            ip,
            ports.into_keys().into_iter().collect::<Vec<_>>().join(", ")
        ),
        net::NodeRouting::Routers(routers) => format!("{} routers", routers.len()),
    }
}
