use crate::ARGS;
use crate::KI;
use crate::SETTING;
use actix_web::http::StatusCode;
use actix_web::{HttpRequest, HttpResponse, Path};
use failure::Error;
use log::LevelFilter;
use settings::client::RitaClientSettings;
use settings::FileWrite;

pub fn get_remote_logging(_req: HttpRequest) -> Result<HttpResponse, Error> {
    Ok(HttpResponse::Ok().json(SETTING.get_log().enabled))
}

pub fn remote_logging(path: Path<bool>) -> Result<HttpResponse, Error> {
    let enabled = path.into_inner();
    debug!("/remote_logging/enable/{} hit", enabled);

    SETTING.get_log_mut().enabled = enabled;

    // try and save the config and fail if we can't
    if let Err(e) = SETTING.write().unwrap().write(&ARGS.flag_config) {
        return Err(e);
    }

    if let Err(e) = KI.run_command("/etc/init.d/rita", &["restart"]) {
        return Err(e);
    }

    Ok(HttpResponse::Ok().json(()))
}

pub fn get_remote_logging_level(_req: HttpRequest) -> Result<HttpResponse, Error> {
    let level = &SETTING.get_log().level;
    Ok(HttpResponse::Ok().json(level))
}

pub fn remote_logging_level(path: Path<String>) -> Result<HttpResponse, Error> {
    let level = path.into_inner();
    debug!("/remote_logging/level/{}", level);

    let log_level: LevelFilter = match level.parse() {
        Ok(level) => level,
        Err(e) => {
            return Ok(HttpResponse::new(StatusCode::BAD_REQUEST)
                .into_builder()
                .json(format!("Could not parse loglevel {:?}", e)));
        }
    };

    SETTING.get_log_mut().level = log_level.to_string();

    // try and save the config and fail if we can't
    if let Err(e) = SETTING.write().unwrap().write(&ARGS.flag_config) {
        return Err(e);
    }

    if let Err(e) = KI.run_command("/etc/init.d/rita", &["restart"]) {
        return Err(e);
    }

    Ok(HttpResponse::Ok().json(()))
}
