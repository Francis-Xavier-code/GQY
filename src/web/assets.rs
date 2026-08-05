//! Embedded static frontend assets.
use axum::http::header::{
    CACHE_CONTROL, CONTENT_SECURITY_POLICY, CONTENT_TYPE, REFERRER_POLICY, X_CONTENT_TYPE_OPTIONS,
};
use axum::http::HeaderValue;
use axum::response::{IntoResponse, Response};

pub(crate) const INDEX_HTML: &str = include_str!("../../web/index.html");
pub(crate) const STYLES_CSS: &str = include_str!("../../web/styles.css");
pub(crate) const APP_JS: &str = include_str!("../../web/app.js");
pub(crate) const USAGE_VIZ_JS: &str = include_str!("../../web/usage-viz.js");
pub(crate) const GQY_LOGO: &[u8] = include_bytes!("../../pics/GQY-avatar.png");
pub(crate) const GQY_WALLPAPER: &[u8] = include_bytes!("../../pics/GQY-image.png");
pub(crate) const PROVIDER_ICONS: &str = include_str!("../../web/assets/provider-icons.svg");

pub(crate) async fn index_asset() -> Response {
    text_asset(INDEX_HTML, "text/html; charset=utf-8")
}

pub(crate) async fn styles_asset() -> Response {
    text_asset(STYLES_CSS, "text/css; charset=utf-8")
}

pub(crate) async fn app_asset() -> Response {
    text_asset(APP_JS, "application/javascript; charset=utf-8")
}

pub(crate) async fn usage_viz_asset() -> Response {
    text_asset(USAGE_VIZ_JS, "application/javascript; charset=utf-8")
}

pub(crate) async fn logo_asset() -> Response {
    binary_asset(GQY_LOGO, "image/png")
}

pub(crate) async fn wallpaper_asset() -> Response {
    binary_asset(GQY_WALLPAPER, "image/png")
}

pub(crate) async fn provider_icons_asset() -> Response {
    text_asset(PROVIDER_ICONS, "image/svg+xml; charset=utf-8")
}

pub(crate) fn text_asset(content: &'static str, content_type: &'static str) -> Response {
    asset_response(content.as_bytes(), content_type)
}

pub(crate) fn binary_asset(content: &'static [u8], content_type: &'static str) -> Response {
    asset_response(content, content_type)
}

pub(crate) fn asset_response(content: &'static [u8], content_type: &'static str) -> Response {
    let mut response = content.into_response();
    response
        .headers_mut()
        .insert(CONTENT_TYPE, HeaderValue::from_static(content_type));
    response
        .headers_mut()
        .insert(CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    response
        .headers_mut()
        .insert(X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    response.headers_mut().insert(
        CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(
            "default-src 'self'; img-src 'self'; style-src 'self'; script-src 'self'; connect-src 'self'; base-uri 'none'; frame-ancestors 'none'; form-action 'self'",
        ),
    );
    response
        .headers_mut()
        .insert(REFERRER_POLICY, HeaderValue::from_static("no-referrer"));
    response
}

