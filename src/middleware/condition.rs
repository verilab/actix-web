//! `Middleware` for conditionally enables another middleware.
use core::future::Future;
use core::task::{Context, Poll};

use actix_service::{Service, Transform};

/// `Middleware` for conditionally enables another middleware.
/// The controlled middleware must not change the `Service` interfaces.
/// This means you cannot control such middlewares like `Logger` or `Compress`.
///
/// ## Usage
///
/// ```rust
/// use actix_web::middleware::{Condition, NormalizePath};
/// use actix_web::App;
///
/// # fn main() {
/// let enable_normalize = std::env::var("NORMALIZE_PATH") == Ok("true".into());
/// let app = App::new()
///     .wrap(Condition::new(enable_normalize, NormalizePath::default()));
/// # }
/// ```
pub struct Condition<T> {
    trans: T,
    enable: bool,
}

impl<T> Condition<T> {
    pub fn new(enable: bool, trans: T) -> Self {
        Self { trans, enable }
    }
}

impl<S, T> Transform<S> for Condition<T>
where
    S: Service + 'static,
    T: Transform<S, Request = S::Request, Response = S::Response, Error = S::Error>,
    T::Future: 'static,
    T::InitError: 'static,
    T::Transform: 'static,
{
    type Request = S::Request;
    type Response = S::Response;
    type Error = S::Error;
    type Transform = ConditionMiddleware<T::Transform, S>;
    type InitError = T::InitError;
    type Future = impl Future<Output = Result<Self::Transform, Self::InitError>>;

    fn new_transform(&self, service: S) -> Self::Future {
        let mut left = None;
        let mut right = None;

        if self.enable {
            left = Some(self.trans.new_transform(service));
        } else {
            right = Some(Ok(ConditionMiddleware::Disable(service)))
        }

        async move {
            match left {
                Some(fut) => {
                    let res = fut.await?;
                    Ok(ConditionMiddleware::Enable(res))
                }
                None => right.unwrap(),
            }
        }
    }
}

pub enum ConditionMiddleware<E, D> {
    Enable(E),
    Disable(D),
}

impl<E, D> Service for ConditionMiddleware<E, D>
where
    E: Service,
    D: Service<Request = E::Request, Response = E::Response, Error = E::Error>,
{
    type Request = E::Request;
    type Response = E::Response;
    type Error = E::Error;
    type Future = impl Future<Output = Result<Self::Response, Self::Error>>;

    fn poll_ready(&self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        use ConditionMiddleware::*;
        match self {
            Enable(service) => service.poll_ready(cx),
            Disable(service) => service.poll_ready(cx),
        }
    }

    fn call(&self, req: E::Request) -> Self::Future {
        let mut left = None;
        let mut right = None;

        match self {
            Self::Enable(service) => left = Some(service.call(req)),
            Self::Disable(service) => right = Some(service.call(req)),
        }

        async move {
            match left {
                Some(left) => left.await,
                None => right.unwrap().await,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use core::future::ready;

    use actix_service::IntoService;

    use super::*;
    use crate::dev::{ServiceRequest, ServiceResponse};
    use crate::error::Result;
    use crate::http::{header::CONTENT_TYPE, HeaderValue, StatusCode};
    use crate::middleware::errhandlers::*;
    use crate::test::{self, TestRequest};
    use crate::HttpResponse;

    #[allow(clippy::unnecessary_wraps)]
    fn render_500<B>(mut res: ServiceResponse<B>) -> Result<ErrorHandlerResponse<B>> {
        res.response_mut()
            .headers_mut()
            .insert(CONTENT_TYPE, HeaderValue::from_static("0001"));
        Ok(ErrorHandlerResponse::Response(res))
    }

    #[actix_rt::test]
    async fn test_handler_enabled() {
        let srv = |req: ServiceRequest| {
            ready(Ok(
                req.into_response(HttpResponse::InternalServerError().finish())
            ))
        };

        let mw =
            ErrorHandlers::new().handler(StatusCode::INTERNAL_SERVER_ERROR, render_500);

        let mut mw = Condition::new(true, mw)
            .new_transform(srv.into_service())
            .await
            .unwrap();
        let resp =
            test::call_service(&mut mw, TestRequest::default().to_srv_request()).await;
        assert_eq!(resp.headers().get(CONTENT_TYPE).unwrap(), "0001");
    }

    #[actix_rt::test]
    async fn test_handler_disabled() {
        let srv = |req: ServiceRequest| {
            ready(Ok(
                req.into_response(HttpResponse::InternalServerError().finish())
            ))
        };

        let mw =
            ErrorHandlers::new().handler(StatusCode::INTERNAL_SERVER_ERROR, render_500);

        let mut mw = Condition::new(false, mw)
            .new_transform(srv.into_service())
            .await
            .unwrap();

        let resp =
            test::call_service(&mut mw, TestRequest::default().to_srv_request()).await;
        assert_eq!(resp.headers().get(CONTENT_TYPE), None);
    }
}
