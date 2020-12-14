use std::{env, io};

use actix_http::{Error, HttpService, Request, Response};
use actix_rt::net::TcpStream;
use actix_server::{ServerBuilder, SingleThreadServer};
use bytes::BytesMut;
use futures_util::StreamExt;
use http::header::HeaderValue;
use log::info;

#[actix_rt::main]
async fn main() -> io::Result<()> {
    env::set_var("RUST_LOG", "echo=info");
    env_logger::init();

    SingleThreadServer::build()
        .bind::<_, _, _, TcpStream>("echo", "127.0.0.1:8080", || {
            HttpService::build()
                .client_timeout(1000)
                .client_disconnect(1000)
                .finish(|mut req: Request| async move {
                    let mut body = BytesMut::new();
                    while let Some(item) = req.payload().next().await {
                        body.extend_from_slice(&item?);
                    }

                    info!("request body: {:?}", body);
                    Ok::<_, Error>(
                        Response::Ok()
                            .header("x-head", HeaderValue::from_static("dummy value!"))
                            .body(body),
                    )
                })
                .tcp()
        })?
        .run()?
        .await
}
