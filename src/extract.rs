//! Request extractors
use std::future::{ready, Future, Ready};

use actix_http::error::Error;

use crate::dev::Payload;
use crate::request::HttpRequest;

/// Trait implemented by types that can be extracted from request.
///
/// Types that implement this trait can be used with `Route` handlers.
pub trait FromRequest: Sized {
    /// The associated error which can be returned.
    type Error: Into<Error>;

    /// Future that resolves to a Self
    type Future<'f>: Future<Output = Result<Self, Self::Error>>;

    /// Configuration for this extractor
    type Config: Default + 'static;

    /// Convert request to a Self
    fn from_request<'a>(
        req: &'a HttpRequest,
        payload: &'a mut Payload,
    ) -> Self::Future<'a>;

    // /// Convert request to a Self
    // ///
    // /// This method uses `Payload::None` as payload stream.
    // fn extract(req: &HttpRequest) -> Self::FutureEtract<'_> {
    //     let payload = &mut Payload::None;
    //     async move {
    //         Self::from_request(req, payload).await
    //     }
    // }

    /// Create and configure config instance.
    fn configure<F>(f: F) -> Self::Config
    where
        F: FnOnce(Self::Config) -> Self::Config,
    {
        f(Self::Config::default())
    }
}

/// Optionally extract a field from the request
///
/// If the FromRequest for T fails, return None rather than returning an error response
///
/// ## Example
///
/// ```rust
/// #![feature(generic_associated_types)]
///
/// use actix_web::{web, dev, App, Error, HttpRequest, FromRequest};
/// use actix_web::error::ErrorBadRequest;
/// use futures_util::future::{ok, err, Ready};
/// use serde_derive::Deserialize;
/// use rand;
///
/// #[derive(Debug, Deserialize)]
/// struct Thing {
///     name: String
/// }
///
/// impl FromRequest for Thing {
///     type Error = Error;
///     type Future<'f> = Ready<Result<Self, Self::Error>>;
///     type Config = ();
///
///     fn from_request<'a>(req: &'a HttpRequest, payload: &'a mut dev::Payload) -> Self::Future<'a> {
///         if rand::random() {
///             ok(Thing { name: "thingy".into() })
///         } else {
///             err(ErrorBadRequest("no luck"))
///         }
///
///     }
/// }
///
/// /// extract `Thing` from request
/// async fn index(supplied_thing: Option<Thing>) -> String {
///     match supplied_thing {
///         // Puns not intended
///         Some(thing) => format!("Got something: {:?}", thing),
///         None => format!("No thing!")
///     }
/// }
///
/// fn main() {
///     let app = App::new().service(
///         web::resource("/users/:first").route(
///             web::post().to(index))
///     );
/// }
/// ```
impl<T: FromRequest> FromRequest for Option<T> {
    type Error = Error;
    type Future<'f> = impl Future<Output = Result<Self, Error>>;
    type Config = T::Config;

    #[inline]
    fn from_request<'a>(
        req: &'a HttpRequest,
        payload: &'a mut Payload,
    ) -> Self::Future<'a> {
        async move {
            match T::from_request(req, payload).await {
                Ok(t) => Ok(Some(t)),
                Err(e) => {
                    log::debug!("Error for Option<T> extractor: {}", e.into());
                    Ok(None)
                }
            }
        }
    }
}

/// Optionally extract a field from the request or extract the Error if unsuccessful
///
/// If the `FromRequest` for T fails, inject Err into handler rather than returning an error response
///
/// ## Example
///
/// ```rust
/// #![feature(generic_associated_types)]
///
/// use actix_web::{web, dev, App, Result, Error, HttpRequest, FromRequest};
/// use actix_web::error::ErrorBadRequest;
/// use futures_util::future::{ok, err, Ready};
/// use serde_derive::Deserialize;
/// use rand;
///
/// #[derive(Debug, Deserialize)]
/// struct Thing {
///     name: String
/// }
///
/// impl FromRequest for Thing {
///     type Error = Error;
///     type Future<'f> = Ready<Result<Thing, Error>>;
///     type Config = ();
///
///     fn from_request<'a>(req: &'a HttpRequest, payload: &'a mut dev::Payload) -> Self::Future<'a> {
///         if rand::random() {
///             ok(Thing { name: "thingy".into() })
///         } else {
///             err(ErrorBadRequest("no luck"))
///         }
///     }
/// }
///
/// /// extract `Thing` from request
/// async fn index(supplied_thing: Result<Thing>) -> String {
///     match supplied_thing {
///         Ok(thing) => format!("Got thing: {:?}", thing),
///         Err(e) => format!("Error extracting thing: {}", e)
///     }
/// }
///
/// fn main() {
///     let app = App::new().service(
///         web::resource("/users/:first").route(web::post().to(index))
///     );
/// }
/// ```
impl<T: FromRequest> FromRequest for Result<T, T::Error> {
    type Error = Error;
    type Future<'f> = impl Future<Output = Result<Self, Error>>;
    type Config = T::Config;

    #[inline]
    fn from_request<'a>(
        req: &'a HttpRequest,
        payload: &'a mut Payload,
    ) -> Self::Future<'a> {
        async move {
            let res = T::from_request(req, payload).await;
            Ok(res)
        }
    }
}

#[doc(hidden)]
impl FromRequest for () {
    type Error = Error;
    type Future<'f> = Ready<Result<(), Error>>;
    type Config = ();

    fn from_request<'a>(_: &'a HttpRequest, _: &'a mut Payload) -> Self::Future<'a> {
        ready(Ok(()))
    }
}

macro_rules! tuple_from_req ({$fut_type:ident, $(($n:tt, $T:ident)),+} => {

    // This module is a trick to get around the inability of
    // `macro_rules!` macros to make new idents. We want to make
    // a new `FutWrapper` struct for each distinct invocation of
    // this macro. Ideally, we would name it something like
    // `FutWrapper_$fut_type`, but this can't be done in a macro_rules
    // macro.
    //
    // Instead, we put everything in a module named `$fut_type`, thus allowing
    // us to use the name `FutWrapper` without worrying about conflicts.
    // This macro only exists to generate trait impls for tuples - these
    // are inherently global, so users don't have to care about this
    // weird trick.
    #[allow(non_snake_case)]
    mod $fut_type {
        // Bring everything into scope, so we don't need
        // redundant imports
        use core::future::Future;

        use actix_http::error::Error;

        use super::FromRequest;

        use crate::dev::Payload;
        use crate::request::HttpRequest;

        /// FromRequest implementation for tuple
        #[doc(hidden)]
        #[allow(unused_parens)]
        impl<$($T: FromRequest + 'static),+> FromRequest for ($($T,)+) {
            type Error = Error;
            type Future<'f> = impl Future<Output = Result<($($T,)+), Self::Error>>;
            type Config = ($($T::Config),+);

            fn from_request<'a>(req: &'a HttpRequest, payload: &'a mut Payload) -> Self::Future<'a> {
                let mut items = <($(Option<$T>,)+)>::default();

                async move {
                    $(
                        let res = $T::from_request(req, payload).await.map_err(Into::into)?;
                        items.$n = Some(res);
                    )+

                    Ok(($(items.$n.take().unwrap(),)+))
                }
            }
        }
    }
});

#[rustfmt::skip]
mod m {
    use super::*;

tuple_from_req!(TupleFromRequest1, (0, A));
tuple_from_req!(TupleFromRequest2, (0, A), (1, B));
tuple_from_req!(TupleFromRequest3, (0, A), (1, B), (2, C));
tuple_from_req!(TupleFromRequest4, (0, A), (1, B), (2, C), (3, D));
tuple_from_req!(TupleFromRequest5, (0, A), (1, B), (2, C), (3, D), (4, E));
tuple_from_req!(TupleFromRequest6, (0, A), (1, B), (2, C), (3, D), (4, E), (5, F));
tuple_from_req!(TupleFromRequest7, (0, A), (1, B), (2, C), (3, D), (4, E), (5, F), (6, G));
tuple_from_req!(TupleFromRequest8, (0, A), (1, B), (2, C), (3, D), (4, E), (5, F), (6, G), (7, H));
tuple_from_req!(TupleFromRequest9, (0, A), (1, B), (2, C), (3, D), (4, E), (5, F), (6, G), (7, H), (8, I));
tuple_from_req!(TupleFromRequest10, (0, A), (1, B), (2, C), (3, D), (4, E), (5, F), (6, G), (7, H), (8, I), (9, J));
}

#[cfg(test)]
mod tests {
    use actix_http::http::header;
    use bytes::Bytes;
    use serde_derive::Deserialize;

    use super::*;
    use crate::test::TestRequest;
    use crate::types::{Form, FormConfig};

    #[derive(Deserialize, Debug, PartialEq)]
    struct Info {
        hello: String,
    }

    #[actix_rt::test]
    async fn test_option() {
        let (req, mut pl) = TestRequest::with_header(
            header::CONTENT_TYPE,
            "application/x-www-form-urlencoded",
        )
        .data(FormConfig::default().limit(4096))
        .to_http_parts();

        let r = Option::<Form<Info>>::from_request(&req, &mut pl)
            .await
            .unwrap();
        assert_eq!(r, None);

        let (req, mut pl) = TestRequest::with_header(
            header::CONTENT_TYPE,
            "application/x-www-form-urlencoded",
        )
        .header(header::CONTENT_LENGTH, "9")
        .set_payload(Bytes::from_static(b"hello=world"))
        .to_http_parts();

        let r = Option::<Form<Info>>::from_request(&req, &mut pl)
            .await
            .unwrap();
        assert_eq!(
            r,
            Some(Form(Info {
                hello: "world".into()
            }))
        );

        let (req, mut pl) = TestRequest::with_header(
            header::CONTENT_TYPE,
            "application/x-www-form-urlencoded",
        )
        .header(header::CONTENT_LENGTH, "9")
        .set_payload(Bytes::from_static(b"bye=world"))
        .to_http_parts();

        let r = Option::<Form<Info>>::from_request(&req, &mut pl)
            .await
            .unwrap();
        assert_eq!(r, None);
    }

    #[actix_rt::test]
    async fn test_result() {
        let (req, mut pl) = TestRequest::with_header(
            header::CONTENT_TYPE,
            "application/x-www-form-urlencoded",
        )
        .header(header::CONTENT_LENGTH, "11")
        .set_payload(Bytes::from_static(b"hello=world"))
        .to_http_parts();

        let r = Result::<Form<Info>, Error>::from_request(&req, &mut pl)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            r,
            Form(Info {
                hello: "world".into()
            })
        );

        let (req, mut pl) = TestRequest::with_header(
            header::CONTENT_TYPE,
            "application/x-www-form-urlencoded",
        )
        .header(header::CONTENT_LENGTH, "9")
        .set_payload(Bytes::from_static(b"bye=world"))
        .to_http_parts();

        let r = Result::<Form<Info>, Error>::from_request(&req, &mut pl)
            .await
            .unwrap();
        assert!(r.is_err());
    }
}
