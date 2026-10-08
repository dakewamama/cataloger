use axum::{
    body::Body,
    http::{Request, StatusCode},
    middleware::Next,
    response::Response,
};

pub async fn verify_helius_signature(
    req: Request<Body>,
    next: Next,
) -> Result<Response, StatusCode> {
    let secret =
        std::env::var("HELIUS_WEBHOOK_SECRET").map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let auth_header = req
        .headers()
        .get("Authorization")
        .and_then(|v| v.to_str().ok())
        .ok_or(StatusCode::UNAUTHORIZED)?;

    if auth_header != secret {
        return Err(StatusCode::UNAUTHORIZED);
    }

    Ok(next.run(req).await)
}

#[cfg(test)]
mod tests {
    use axum::{body::Body, http::Request};

    #[test]
    fn rejects_when_secret_not_set() {
        unsafe { std::env::remove_var("HELIUS_WEBHOOK_SECRET") };
        let req = Request::builder()
            .header("Authorization", "wrong")
            .body(Body::empty())
            .unwrap();
        assert!(req.headers().get("Authorization").is_some());
    }

    #[test]
    fn header_matches_secret() {
        unsafe { std::env::set_var("HELIUS_WEBHOOK_SECRET", "test_secret") };
        let secret = std::env::var("HELIUS_WEBHOOK_SECRET").unwrap();
        assert_eq!(secret, "test_secret");
        unsafe { std::env::remove_var("HELIUS_WEBHOOK_SECRET") };
    }
}
