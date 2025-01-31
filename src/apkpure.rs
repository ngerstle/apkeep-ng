use std::collections::HashSet;
use std::ops::Deref;
use std::path::Path;
use std::rc::Rc;

use futures_util::StreamExt;
use regex::Regex;
use reqwest::header::{HeaderMap, HeaderValue};
use reqwest::{Url, Response};
use tokio_dl_stream_to_disk::error::ErrorKind as TDSTDErrorKind;
use tokio::time::{sleep, Duration as TokioDuration};

fn http_headers() -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert("x-cv", HeaderValue::from_static("3172501"));
    headers.insert("x-sv", HeaderValue::from_static("29"));
    headers.insert(
        "x-abis",
        HeaderValue::from_static("arm64-v8a,armeabi-v7a,armeabi"),
    );
    headers.insert("x-gp", HeaderValue::from_static("1"));
    headers
}

pub async fn download_apps(
    apps: Vec<(String, Option<String>)>,
    parallel: usize,
    sleep_duration: u64,
    outpath: &Path,
) {
    let http_client = Rc::new(reqwest::Client::new());
    let headers = http_headers();
    let re = Rc::new(Regex::new(crate::consts::APKPURE_DOWNLOAD_URL_REGEX).unwrap());

    futures_util::stream::iter(
        apps.into_iter().map(|app| {
            let (app_id, app_version) = app;
            let http_client = Rc::clone(&http_client);
            let re = Rc::clone(&re);
            let headers = headers.clone();
            async move {
                let app_string = match app_version {
                    Some(ref version) => {
                        println!("Downloading {} version {}...", app_id, version);
                        format!("{}@{}", app_id, version)
                    },
                    None => {
                        println!("Downloading {}...", app_id);
                        app_id.to_string()
                    },
                };
                if sleep_duration > 0 {
                    sleep(TokioDuration::from_millis(sleep_duration)).await;
                }
                let versions_url = Url::parse(&format!("{}{}", crate::consts::APKPURE_VERSIONS_URL_FORMAT, app_id)).unwrap();
                let versions_response = http_client
                    .get(versions_url)
                    .headers(headers)
                    .send().await.unwrap();
                if let Some(app_version) = app_version {
                    let regex_string = format!("[[:^digit:]]{}:(?s:.)+?{}", regex::escape(&app_version), crate::consts::APKPURE_DOWNLOAD_URL_REGEX);
                    let re = Regex::new(&regex_string).unwrap();
                    download_from_response(versions_response, Box::new(Box::new(re)), app_string, outpath).await;
                } else {
                    download_from_response(versions_response, Box::new(re), app_string, outpath).await;
                }
            }
        })
    ).buffer_unordered(parallel).collect::<Vec<()>>().await;
}

async fn download_from_response(response: Response, re: Box<dyn Deref<Target=Regex>>, app_string: String, outpath: &Path) {
    let fname = format!("{}.apk", app_string);
    match response.status() {
        reqwest::StatusCode::OK => {
            let body = response.text().await.unwrap();
            match re.captures(&body) {
                Some(caps) if caps.len() >= 2 => {
                    let download_url = caps.get(1).unwrap().as_str();
                    match tokio_dl_stream_to_disk::download(download_url, Path::new(outpath), &fname).await {
                        Ok(_) => println!("{} downloaded successfully!", app_string),
                        Err(err) if matches!(err.kind(), TDSTDErrorKind::FileExists) => {
                            println!("File already exists for {}. Skipping...", app_string);
                        },
                        Err(err) if matches!(err.kind(), TDSTDErrorKind::PermissionDenied) => {
                            println!("Permission denied when attempting to write file for {}. Skipping...", app_string);
                        },
                        Err(_) => {
                            println!("An error has occurred attempting to download {}.  Retry #1...", app_string);
                            match tokio_dl_stream_to_disk::download(download_url, Path::new(outpath), &fname).await {
                                Ok(_) => println!("{} downloaded successfully!", app_string),
                                Err(_) => {
                                    println!("An error has occurred attempting to download {}.  Retry #2...", app_string);
                                    match tokio_dl_stream_to_disk::download(download_url, Path::new(outpath), &fname).await {
                                        Ok(_) => println!("{} downloaded successfully!", app_string),
                                        Err(_) => {
                                            println!("An error has occurred attempting to download {}. Skipping...", app_string);
                                        }
                                    }
                                }
                            }
                        }
                    }
                },
                _ => {
                    println!("Could not get download URL for {}. Skipping...", app_string);
                }
            }

        },
        _ => {
            println!("Invalid app response for {}. Skipping...", app_string);
        }
    }
}

pub async fn list_versions(apps: Vec<(String, Option<String>)>) {
    let http_client = Rc::new(reqwest::Client::new());
    let re = Rc::new(Regex::new(r"([[:alnum:]\.-]+):\([[:xdigit:]]{40,}").unwrap());
    let headers = http_headers();
    for app in apps {
        let (app_id, _) = app;
        let http_client = Rc::clone(&http_client);
        let re = Rc::clone(&re);
        let headers = headers.clone();
        async move {
            println!("Versions available for {} on APKPure:", app_id);
            let versions_url = Url::parse(&format!("{}{}", crate::consts::APKPURE_VERSIONS_URL_FORMAT, app_id)).unwrap();
            let versions_response = http_client
                .get(versions_url)
                .headers(headers)
                .send().await.unwrap();

            match versions_response.status() {
                reqwest::StatusCode::OK => {
                    let body = versions_response.text().await.unwrap();
                    let mut versions = HashSet::new();
                    for caps in re.captures_iter(&body) {
                        if caps.len() >= 2 {
                            versions.insert(caps.get(1).unwrap().as_str().to_string());
                        }
                    }
                    let mut versions = versions.drain().collect::<Vec<String>>();
                    versions.sort();
                    println!("| {}", versions.join(", "));
                }
                _ => {
                    println!("| Invalid app response for {}. Skipping...", app_id);
                }
            }
        }.await;
    }
}
