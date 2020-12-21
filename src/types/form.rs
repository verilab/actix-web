//! Form extractor

use core::future::{ready, Future, Ready};
use core::pin::Pin;
use core::task::{Context, Poll};
use core::{fmt, ops};

use std::rc::Rc;

use actix_http::{Error, HttpMessage, Payload, Response};
use bytes::BytesMut;
use encoding_rs::{Encoding, UTF_8};
use futures_core::ready;
use futures_core::stream::Stream;
use serde::de::DeserializeOwned;
use serde::Serialize;

#[cfg(feature = "compress")]
use crate::dev::Decompress;
use crate::error::UrlencodedError;
use crate::extract::FromRequest;
use crate::http::{
    header::{ContentType, CONTENT_LENGTH},
    StatusCode,
};
use crate::request::HttpRequest;
use crate::{responder::Responder, web};
use std::marker::PhantomData;

/// Form data helper (`application/x-www-form-urlencoded`)
///
/// Can be use to extract url-encoded data from the request body,
/// or send url-encoded data as the response.
///
/// ## Extract
///
/// To extract typed information from request's body, the type `T` must
/// implement the `Deserialize` trait from *serde*.
///
/// [**FormConfig**](FormConfig) allows to configure extraction
/// process.
///
/// ### Example
/// ```rust
/// use actix_web::web;
/// use serde_derive::Deserialize;
///
/// #[derive(Deserialize)]
/// struct FormData {
///     username: String,
/// }
///
/// /// Extract form data using serde.
/// /// This handler get called only if content type is *x-www-form-urlencoded*
/// /// and content of the request could be deserialized to a `FormData` struct
/// fn index(form: web::Form<FormData>) -> String {
///     format!("Welcome {}!", form.username)
/// }
/// # fn main() {}
/// ```
///
/// ## Respond
///
/// The `Form` type also allows you to respond with well-formed url-encoded data:
/// simply return a value of type Form<T> where T is the type to be url-encoded.
/// The type  must implement `serde::Serialize`;
///
/// ### Example
/// ```rust
/// use actix_web::*;
/// use serde_derive::Serialize;
///
/// #[derive(Serialize)]
/// struct SomeForm {
///     name: String,
///     age: u8
/// }
///
/// // Will return a 200 response with header
/// // `Content-Type: application/x-www-form-urlencoded`
/// // and body "name=actix&age=123"
/// fn index() -> web::Form<SomeForm> {
///     web::Form(SomeForm {
///         name: "actix".into(),
///         age: 123
///     })
/// }
/// # fn main() {}
/// ```
#[derive(PartialEq, Eq, PartialOrd, Ord)]
pub struct Form<T>(pub T);

impl<T> Form<T> {
    /// Deconstruct to an inner value
    pub fn into_inner(self) -> T {
        self.0
    }
}

impl<T> ops::Deref for Form<T> {
    type Target = T;

    fn deref(&self) -> &T {
        &self.0
    }
}

impl<T> ops::DerefMut for Form<T> {
    fn deref_mut(&mut self) -> &mut T {
        &mut self.0
    }
}

impl<T> FromRequest for Form<T>
where
    T: DeserializeOwned + 'static,
{
    type Error = Error;
    type Future = impl Future<Output = Result<Self, Error>>;
    type Config = FormConfig;

    #[inline]
    fn from_request(req: &HttpRequest, payload: &mut Payload) -> Self::Future {
        let (limit, err) = req
            .app_data::<Self::Config>()
            .or_else(|| {
                req.app_data::<web::Data<Self::Config>>()
                    .map(|d| d.as_ref())
            })
            .map(|c| (c.limit, c.err_handler.clone()))
            .unwrap_or((16384, None));

        let fut = UrlEncoded::new(req, payload).limit(limit);
        let req = req.clone();

        async move {
            match fut.await {
                Ok(item) => Ok(Form(item)),
                Err(e) => {
                    if let Some(err) = err {
                        Err((*err)(e, &req))
                    } else {
                        Err(e.into())
                    }
                }
            }
        }
    }
}

impl<T: fmt::Debug> fmt::Debug for Form<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl<T: fmt::Display> fmt::Display for Form<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl<T: Serialize> Responder for Form<T> {
    type Error = Error;
    type Future = Ready<Result<Response, Error>>;

    fn respond_to(self, _: &HttpRequest) -> Self::Future {
        let body = match serde_urlencoded::to_string(&self.0) {
            Ok(body) => body,
            Err(e) => return ready(Err(e.into())),
        };

        ready(Ok(Response::build(StatusCode::OK)
            .set(ContentType::form_url_encoded())
            .body(body)))
    }
}

/// Form extractor configuration
///
/// ```rust
/// use actix_web::{web, App, FromRequest, Result};
/// use serde_derive::Deserialize;
///
/// #[derive(Deserialize)]
/// struct FormData {
///     username: String,
/// }
///
/// /// Extract form data using serde.
/// /// Custom configuration is used for this handler, max payload size is 4k
/// async fn index(form: web::Form<FormData>) -> Result<String> {
///     Ok(format!("Welcome {}!", form.username))
/// }
///
/// fn main() {
///     let app = App::new().service(
///         web::resource("/index.html")
///             // change `Form` extractor configuration
///             .app_data(
///                 web::FormConfig::default().limit(4097)
///             )
///             .route(web::get().to(index))
///     );
/// }
/// ```
#[derive(Clone)]
pub struct FormConfig {
    limit: usize,
    err_handler: Option<Rc<dyn Fn(UrlencodedError, &HttpRequest) -> Error>>,
}

impl FormConfig {
    /// Change max size of payload. By default max size is 16Kb
    pub fn limit(mut self, limit: usize) -> Self {
        self.limit = limit;
        self
    }

    /// Set custom error handler
    pub fn error_handler<F>(mut self, f: F) -> Self
    where
        F: Fn(UrlencodedError, &HttpRequest) -> Error + 'static,
    {
        self.err_handler = Some(Rc::new(f));
        self
    }
}

impl Default for FormConfig {
    fn default() -> Self {
        FormConfig {
            limit: 16_384, // 2^14 bytes (~16kB)
            err_handler: None,
        }
    }
}

/// Future that resolves to a parsed urlencoded values.
///
/// Parse `application/x-www-form-urlencoded` encoded request's body.
/// Return `UrlEncoded` future. Form can be deserialized to any type that
/// implements `Deserialize` trait from *serde*.
///
/// Returns error:
///
/// * content type is not `application/x-www-form-urlencoded`
/// * content-length is greater than 32k
///
pub enum UrlEncoded<U> {
    Error(Option<UrlencodedError>),
    Body {
        #[cfg(feature = "compress")]
        stream: Decompress<Payload>,
        #[cfg(not(feature = "compress"))]
        stream: Payload,
        limit: usize,
        length: Option<usize>,
        encoding: &'static Encoding,
        buf: BytesMut,
        _res: PhantomData<U>,
    },
}

impl<U> Unpin for UrlEncoded<U> {}

#[allow(clippy::borrow_interior_mutable_const)]
impl<U> UrlEncoded<U> {
    /// Create a new future to URL encode a request
    pub fn new(req: &HttpRequest, payload: &mut Payload) -> UrlEncoded<U> {
        // check content type
        if req.content_type().to_lowercase() != "application/x-www-form-urlencoded" {
            return Self::Error(Some(UrlencodedError::ContentType));
        }

        let encoding = match req.encoding() {
            Ok(enc) => enc,
            Err(_) => return Self::Error(Some(UrlencodedError::ContentType)),
        };

        let mut len = None;
        let limit = 32_768;

        if let Some(l) = req.headers().get(&CONTENT_LENGTH) {
            if let Ok(s) = l.to_str() {
                if let Ok(l) = s.parse::<usize>() {
                    if l > limit {
                        return Self::Error(Some(UrlencodedError::Overflow {
                            size: l,
                            limit,
                        }));
                    }
                    len = Some(l)
                } else {
                    return Self::Error(Some(UrlencodedError::UnknownLength));
                }
            } else {
                return Self::Error(Some(UrlencodedError::UnknownLength));
            }
        };

        #[cfg(feature = "compress")]
        let payload = Decompress::from_headers(payload.take(), req.headers());
        #[cfg(not(feature = "compress"))]
        let payload = payload.take();

        UrlEncoded::Body {
            encoding,
            stream: payload,
            limit,
            length: len,
            buf: BytesMut::with_capacity(8192),
            _res: PhantomData,
        }
    }

    /// Change max size of payload. By default max size is 256Kb
    pub fn limit(self, limit: usize) -> Self {
        match self {
            UrlEncoded::Body {
                encoding,
                stream,
                length,
                buf,
                ..
            } => {
                if let Some(len) = length {
                    if len > limit {
                        return UrlEncoded::Error(Some(UrlencodedError::Overflow {
                            size: len,
                            limit,
                        }));
                    }
                }
                UrlEncoded::Body {
                    stream,
                    limit,
                    length,
                    encoding,
                    buf,
                    _res: PhantomData,
                }
            }
            _ => self,
        }
    }
}

impl<U> Future for UrlEncoded<U>
where
    U: DeserializeOwned + 'static,
{
    type Output = Result<U, UrlencodedError>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        match this {
            UrlEncoded::Error(e) => Poll::Ready(Err(e.take().unwrap())),
            UrlEncoded::Body {
                stream,
                limit,
                encoding,
                buf,
                ..
            } => loop {
                match ready!(Pin::new(&mut *stream).poll_next(cx)) {
                    Some(chunk) => {
                        let chunk = chunk?;
                        if (buf.len() + chunk.len()) > *limit {
                            return Poll::Ready(Err(UrlencodedError::Overflow {
                                size: buf.len() + chunk.len(),
                                limit: *limit,
                            }));
                        } else {
                            buf.extend_from_slice(&chunk);
                        }
                    }
                    None => {
                        let res = if *encoding == UTF_8 {
                            serde_urlencoded::from_bytes::<U>(&buf)
                                .map_err(|_| UrlencodedError::Parse)
                        } else {
                            let body = encoding
                                .decode_without_bom_handling_and_without_replacement(
                                    &buf,
                                )
                                .map(|s| s.into_owned())
                                .ok_or(UrlencodedError::Parse)?;
                            serde_urlencoded::from_str::<U>(&body)
                                .map_err(|_| UrlencodedError::Parse)
                        };
                        return Poll::Ready(res);
                    }
                }
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use bytes::Bytes;
    use serde::{Deserialize, Serialize};

    use super::*;
    use crate::http::header::{HeaderValue, CONTENT_LENGTH, CONTENT_TYPE};
    use crate::test::TestRequest;

    #[derive(Deserialize, Serialize, Debug, PartialEq)]
    struct Info {
        hello: String,
        counter: i64,
    }

    #[actix_rt::test]
    async fn test_form() {
        let (req, mut pl) =
            TestRequest::with_header(CONTENT_TYPE, "application/x-www-form-urlencoded")
                .header(CONTENT_LENGTH, "11")
                .set_payload(Bytes::from_static(b"hello=world&counter=123"))
                .to_http_parts();

        let Form(s) = Form::<Info>::from_request(&req, &mut pl).await.unwrap();
        assert_eq!(
            s,
            Info {
                hello: "world".into(),
                counter: 123
            }
        );
    }

    fn eq(err: UrlencodedError, other: UrlencodedError) -> bool {
        match err {
            UrlencodedError::Overflow { .. } => {
                matches!(other, UrlencodedError::Overflow { .. })
            }
            UrlencodedError::UnknownLength => {
                matches!(other, UrlencodedError::UnknownLength)
            }
            UrlencodedError::ContentType => {
                matches!(other, UrlencodedError::ContentType)
            }
            _ => false,
        }
    }

    #[actix_rt::test]
    async fn test_urlencoded_error() {
        let (req, mut pl) =
            TestRequest::with_header(CONTENT_TYPE, "application/x-www-form-urlencoded")
                .header(CONTENT_LENGTH, "xxxx")
                .to_http_parts();
        let info = UrlEncoded::<Info>::new(&req, &mut pl).await;
        assert!(eq(info.err().unwrap(), UrlencodedError::UnknownLength));

        let (req, mut pl) =
            TestRequest::with_header(CONTENT_TYPE, "application/x-www-form-urlencoded")
                .header(CONTENT_LENGTH, "1000000")
                .to_http_parts();
        let info = UrlEncoded::<Info>::new(&req, &mut pl).await;
        assert!(eq(
            info.err().unwrap(),
            UrlencodedError::Overflow { size: 0, limit: 0 }
        ));

        let (req, mut pl) = TestRequest::with_header(CONTENT_TYPE, "text/plain")
            .header(CONTENT_LENGTH, "10")
            .to_http_parts();
        let info = UrlEncoded::<Info>::new(&req, &mut pl).await;
        assert!(eq(info.err().unwrap(), UrlencodedError::ContentType));
    }

    #[actix_rt::test]
    async fn test_urlencoded() {
        let (req, mut pl) =
            TestRequest::with_header(CONTENT_TYPE, "application/x-www-form-urlencoded")
                .header(CONTENT_LENGTH, "11")
                .set_payload(Bytes::from_static(b"hello=world&counter=123"))
                .to_http_parts();

        let info = UrlEncoded::<Info>::new(&req, &mut pl).await.unwrap();
        assert_eq!(
            info,
            Info {
                hello: "world".to_owned(),
                counter: 123
            }
        );

        let (req, mut pl) = TestRequest::with_header(
            CONTENT_TYPE,
            "application/x-www-form-urlencoded; charset=utf-8",
        )
        .header(CONTENT_LENGTH, "11")
        .set_payload(Bytes::from_static(b"hello=world&counter=123"))
        .to_http_parts();

        let info = UrlEncoded::<Info>::new(&req, &mut pl).await.unwrap();
        assert_eq!(
            info,
            Info {
                hello: "world".to_owned(),
                counter: 123
            }
        );
    }

    #[actix_rt::test]
    async fn test_responder() {
        let req = TestRequest::default().to_http_request();

        let form = Form(Info {
            hello: "world".to_string(),
            counter: 123,
        });
        let resp = form.respond_to(&req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            resp.headers().get(CONTENT_TYPE).unwrap(),
            HeaderValue::from_static("application/x-www-form-urlencoded")
        );

        use crate::responder::tests::BodyTest;
        assert_eq!(resp.body().bin_ref(), b"hello=world&counter=123");
    }

    #[actix_rt::test]
    async fn test_with_config_in_data_wrapper() {
        let ctype = HeaderValue::from_static("application/x-www-form-urlencoded");

        let (req, mut pl) = TestRequest::default()
            .header(CONTENT_TYPE, ctype)
            .header(CONTENT_LENGTH, HeaderValue::from_static("20"))
            .set_payload(Bytes::from_static(b"hello=test&counter=4"))
            .app_data(web::Data::new(FormConfig::default().limit(10)))
            .to_http_parts();

        let s = Form::<Info>::from_request(&req, &mut pl).await;
        assert!(s.is_err());

        let err_str = s.err().unwrap().to_string();
        assert!(err_str.contains("Urlencoded payload size is bigger"));
    }
}
