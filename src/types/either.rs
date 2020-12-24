use core::future::Future;

use actix_http::{Error, Response};
use bytes::Bytes;

use crate::{dev, request::HttpRequest, FromRequest, Responder};

/// Combines two different responder types into a single type
///
/// ```rust
/// use actix_web::{Either, Error, HttpResponse};
///
/// type RegisterResult = Either<HttpResponse, Result<HttpResponse, Error>>;
///
/// fn index() -> RegisterResult {
///     if is_a_variant() {
///         // <- choose left variant
///         Either::A(HttpResponse::BadRequest().body("Bad data"))
///     } else {
///         Either::B(
///             // <- Right variant
///             Ok(HttpResponse::Ok()
///                 .content_type("text/html")
///                 .body("Hello!"))
///         )
///     }
/// }
/// # fn is_a_variant() -> bool { true }
/// # fn main() {}
/// ```
#[derive(Debug, PartialEq)]
pub enum Either<A, B> {
    /// First branch of the type
    A(A),
    /// Second branch of the type
    B(B),
}

#[cfg(test)]
impl<A, B> Either<A, B> {
    pub(self) fn unwrap_left(self) -> A {
        match self {
            Either::A(data) => data,
            Either::B(_) => {
                panic!("Cannot unwrap left branch. Either contains a right branch.")
            }
        }
    }

    pub(self) fn unwrap_right(self) -> B {
        match self {
            Either::A(_) => {
                panic!("Cannot unwrap right branch. Either contains a left branch.")
            }
            Either::B(data) => data,
        }
    }
}

impl<A, B> Responder for Either<A, B>
where
    A: Responder,
    B: Responder,
{
    type Error = Error;
    type Future<'f> = impl Future<Output = Result<Response, Self::Error>>;

    fn respond_to(self, req: &HttpRequest) -> Self::Future<'_> {
        async move {
            match self {
                Either::A(a) => a.respond_to(req).await.map_err(Into::into),
                Either::B(b) => b.respond_to(req).await.map_err(Into::into),
            }
        }
    }
}

/// A composite error resulting from failure to extract an `Either<A, B>`.
///
/// The implementation of `Into<actix_web::Error>` will return the payload buffering error or the
/// error from the primary extractor. To access the fallback error, use a match clause.
#[derive(Debug)]
pub enum EitherExtractError<A, B> {
    /// Error from payload buffering, such as exceeding payload max size limit.
    Bytes(Error),

    /// Error from primary extractor.
    Extract(A, B),
}

impl<A, B> Into<Error> for EitherExtractError<A, B>
where
    A: Into<Error>,
    B: Into<Error>,
{
    fn into(self) -> Error {
        match self {
            EitherExtractError::Bytes(err) => err,
            EitherExtractError::Extract(a_err, _b_err) => a_err.into(),
        }
    }
}

/// Provides a mechanism for trying two extractors, a primary and a fallback. Useful for
/// "polymorphic payloads" where, for example, a form might be JSON or URL encoded.
///
/// It is important to note that this extractor, by necessity, buffers the entire request payload
/// as part of its implementation. Though, it does respect a `PayloadConfig`'s maximum size limit.
impl<A, B> FromRequest for Either<A, B>
where
    A: FromRequest,
    B: FromRequest,
{
    type Error = EitherExtractError<A::Error, B::Error>;
    type Future<'f> = impl Future<Output = Result<Self, Self::Error>>;
    type Config = ();

    fn from_request<'a>(
        req: &'a HttpRequest,
        payload: &'a mut dev::Payload,
    ) -> Self::Future<'a> {
        async move {
            let bytes = Bytes::from_request(req, payload)
                .await
                .map_err(EitherExtractError::Bytes)?;
            bytes_to_a_or_b(req, bytes).await
        }
    }
}

async fn bytes_to_a_or_b<A, B>(
    req: &HttpRequest,
    bytes: Bytes,
) -> Result<Either<A, B>, EitherExtractError<A::Error, B::Error>>
where
    A: FromRequest,
    B: FromRequest,
{
    let fallback = bytes.clone();
    let a_err;

    let mut pl = payload_from_bytes(bytes);
    match A::from_request(req, &mut pl).await {
        Ok(a_data) => return Ok(Either::A(a_data)),
        // store A's error for returning if B also fails
        Err(err) => a_err = err,
    };

    let mut pl = payload_from_bytes(fallback);
    match B::from_request(req, &mut pl).await {
        Ok(b_data) => return Ok(Either::B(b_data)),
        Err(b_err) => Err(EitherExtractError::Extract(a_err, b_err)),
    }
}

fn payload_from_bytes(bytes: Bytes) -> dev::Payload {
    let (_, mut h1_payload) = actix_http::h1::Payload::create(true);
    h1_payload.unread_data(bytes);
    dev::Payload::from(h1_payload)
}

#[cfg(test)]
mod tests {
    use serde::{Deserialize, Serialize};

    use super::*;
    use crate::{
        test::TestRequest,
        web::{Form, Json},
    };

    #[derive(Debug, Clone, Serialize, Deserialize)]
    struct TestForm {
        hello: String,
    }

    #[actix_rt::test]
    async fn test_either_extract_first_try() {
        let (req, mut pl) = TestRequest::default()
            .set_form(&TestForm {
                hello: "world".to_owned(),
            })
            .to_http_parts();

        let form = Either::<Form<TestForm>, Json<TestForm>>::from_request(&req, &mut pl)
            .await
            .unwrap()
            .unwrap_left()
            .into_inner();
        assert_eq!(&form.hello, "world");
    }

    #[actix_rt::test]
    async fn test_either_extract_fallback() {
        let (req, mut pl) = TestRequest::default()
            .set_json(&TestForm {
                hello: "world".to_owned(),
            })
            .to_http_parts();

        let form = Either::<Form<TestForm>, Json<TestForm>>::from_request(&req, &mut pl)
            .await
            .unwrap()
            .unwrap_right()
            .into_inner();
        assert_eq!(&form.hello, "world");
    }

    #[actix_rt::test]
    async fn test_either_extract_recursive_fallback() {
        let (req, mut pl) = TestRequest::default()
            .set_payload(Bytes::from_static(b"!@$%^&*()"))
            .to_http_parts();

        let payload =
            Either::<Either<Form<TestForm>, Json<TestForm>>, Bytes>::from_request(
                &req, &mut pl,
            )
            .await
            .unwrap()
            .unwrap_right();
        assert_eq!(&payload.as_ref(), &b"!@$%^&*()");
    }

    #[actix_rt::test]
    async fn test_either_extract_recursive_fallback_inner() {
        let (req, mut pl) = TestRequest::default()
            .set_json(&TestForm {
                hello: "world".to_owned(),
            })
            .to_http_parts();

        let form =
            Either::<Either<Form<TestForm>, Json<TestForm>>, Bytes>::from_request(
                &req, &mut pl,
            )
            .await
            .unwrap()
            .unwrap_left()
            .unwrap_right()
            .into_inner();
        assert_eq!(&form.hello, "world");
    }
}
