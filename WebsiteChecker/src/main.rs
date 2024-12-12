use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};
use std::env;
use std::fs::File;
use std::io::{BufReader, BufRead};
use std::collections::VecDeque;
use std::net::TcpListener;
use std::io::Write;
use chrono::{DateTime, Utc};

//struct to store the status of the website
#[derive(Debug)]
struct WebsiteStatus {
    url: String,
    status: Result<u16, String>,
    response_time: Duration,
    timestamp: DateTime<Utc>,
    headers_valid: bool,
}

//validate the headers of the HTTP response
fn validate_headers(response: &ureq::Response) -> bool {
    //Could not get it to fully work
    //Only checks if the content-type contains "application/json"
    response
        .header("Content-Type")
        .map(|content_type| content_type.to_lowercase().contains("application/json"))
        .unwrap_or(false)
}

//Chech the status of a website/GET requests
pub fn website_checker(url: String, timeout: Duration) -> WebsiteStatus {
    //initalizing the timing
    let start = Instant::now();
    let timestamp = Utc::now();

    //create an agent to make the requests
    let agent = ureq::agent();

    //default Errors for bad results and invalid headers
    let mut response_result = Err("Request error".to_string());
    let mut headers_valid = false;

    //Attempting the GET request
    match agent.get(&url).timeout(timeout).call() {
        Ok(response) => {
            //Stores the status code and checks if the header is valid
            response_result = Ok(response.status());
            headers_valid = validate_headers(&response);
        }
        Err(e) => {
            response_result = Err(format!("Request failed: {} for URL {}", e, url));
        }
    }

    //measures the response time
    let response_time = start.elapsed();
    WebsiteStatus {
        url,
        status: response_result,
        response_time,
        timestamp,
        headers_valid,
    }
}

//monitor the list of websites concurrenly using multiple workers
fn monitor_websites(
    urls: Vec<String>,
    worker_num: usize,
    timeout: Duration,
    retries: usize,
) -> Vec<WebsiteStatus> {
    //create a channel for sending results from workers
    let (sender, receiver) = mpsc::channel();
    
    //mutex protected queue of URLs
    let urls = Arc::new(Mutex::new(urls.into_iter().collect::<VecDeque<String>>()));
    
    //vector to store worker thread handles
    let mut handles = vec![];

    //this creates the worker threads
    for _ in 0..worker_num {
        //creates clones for each thread and their URL queue
        let sender = sender.clone();
        let urls = Arc::clone(&urls);
        let handle = thread::spawn(move || {
            //processes URLs until empty
            while let Some(url) = {
                let mut urls = urls.lock().unwrap();
                urls.pop_front()
            } {
                let mut result = None;

                //retry if it fails
                for _ in 0..=retries {
                    result = Some(website_checker(url.clone(), timeout));
                    if let Some(WebsiteStatus { status: Ok(_), .. }) = result {
                        break;
                    }
                }
                if let Err(e) = sender.send(result.unwrap()) {
                    eprintln!("Failed to send result: {}", e); //CHANGE: Improved error handling.
                }
            }
        });
        //adds it to the vector
        handles.push(handle);
    }

    for handle in handles {
        if let Err(e) = handle.join() {
            eprintln!("Thread join error: {:?}", e); //CHANGE: Handle potential thread join errors.
        }
    }

    //closes the sender channel
    drop(sender);

    receiver.iter().collect()
}

pub fn read_from_file(file_path: &str) -> std::io::Result<Vec<String>> {
    let file = File::open(file_path)?;
    let reader = BufReader::new(file);

    //reads each line from the file and filters out empty lines
    let urls: Vec<String> = reader
        .lines()
        .filter_map(|line| match line {
            Ok(l) if !l.trim().is_empty() => Some(l),
            Ok(_) => None,
            Err(e) => {
                eprintln!("Error reading line: {}", e);
                None
            }
        })
        .collect();

    if urls.is_empty() {
        eprintln!("Warning: No valid URLs found in the file.");
    }

    Ok(urls)
}

//summarizes the results
fn summarize_results(results: &[WebsiteStatus]) {
    let total = results.len();
    let successes = results.iter().filter(|r| r.status.is_ok()).count();
    let failures = total - successes;

    //calcuates average time
    let avg_response_time = results.iter().map(|r| r.response_time).sum::<Duration>() / (total as u32);

    //prints out the summary information
    println!("\nSummary:");
    println!("Total URLs: {}", total);
    println!("Successful: {}", successes);
    println!("Failed: {}", failures);
    println!("Average Response Time: {:?}", avg_response_time);
}

fn main() {
    //collects command line arguments
    let args: Vec<String> = env::args().collect();
    let file_path = "urls.txt";

    //Set up the defaults if not specified
    let worker_num: usize = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(4);
    let timeout: Duration = args.get(2).and_then(|s| s.parse().ok()).map(Duration::from_secs).unwrap_or(Duration::from_secs(5));
    let retries: usize = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(3);

    //reads the urls
    match read_from_file(file_path) {
        Ok(urls) => {
            if urls.is_empty() {
                eprintln!("No URLs found in the file.");
                return;
            }

            //moniters websites and collect results
            let start = Instant::now();
            let results = monitor_websites(urls.clone(), worker_num, timeout, retries);
            for status in &results {
                //print the status of each websit
                println!("{:?}", status);
            }

            summarize_results(&results);

            println!(
                "Total execution time: {:?}",
                Instant::now().duration_since(start)
            );
        }
        Err(e) => {
            eprintln!("Failed to read URLs: {}", e);
        }
    }
}

//gets used in the testing portion
fn start_mock_server(address: &str, response_code: u16, response_body: &str) {
    
    //binds the server to address
    let listener = TcpListener::bind(address).unwrap();
    println!("Mock server running on {}", address);

    for stream in listener.incoming() {
        match stream {
            Ok(mut stream) => {
                let response = format!(
                    //Creates a mock HTTP response
                    "HTTP/1.1 {} OK\r\nContent-Length: {}\r\n\r\n{}",
                    response_code,
                    response_body.len(),
                    response_body
                );
                //Sends the response to the client and then flushes the stream
                stream.write_all(response.as_bytes()).unwrap();
                stream.flush().unwrap();
            }
            Err(e) => {
                eprintln!("Connection failed: {}", e);
            }
        }
    }
}

//starts a mock server in a separate thread
fn mock_server_thread() {
    thread::spawn(move || {
        //start mock with 200 ok response
        start_mock_server("127.0.0.1:8080", 200, "Mock Response");
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use std::time::Instant;
    
    #[test]
    fn test_validate_headers_invalid() {
        // creates a valid response with status code 200 and appropriate headers
        let mock_response = ureq::Response::new(200, "OK", "body content");
    
        // validate the headers
        if let Ok(response) = mock_response {
            // Check that the expected header exists
            // or whatever header is default
            assert_eq!(response.header("Content-Type"), None);
    
            // modify the validation condition for test purposes
            assert!(!validate_headers(&response));
        } else {
            panic!("Failed to create mock response");
        }
    }    

    #[test]
    fn test_website_checker_success() {
        // mock website status with fake response time and status
        let result = website_checker("http://example.com".to_string(), Duration::from_secs(5));

        assert_eq!(result.url, "http://example.com");
        assert!(result.status.is_ok());
        assert!(result.response_time > Duration::from_secs(0));
        assert_eq!(result.headers_valid, false);
    }

    #[test]
    fn test_website_checker_failure() {
        let result = website_checker("http://nonexistent-url.com".to_string(), Duration::from_secs(5));

        assert_eq!(result.url, "http://nonexistent-url.com");
        assert!(result.status.is_err());
    }

    #[test]
    fn test_integration_with_mock_server() {
        // start mock server in a separate thread
        mock_server_thread();

        // simulate checking a website with the mock server
        let start = Instant::now();
        let result = website_checker("http://127.0.0.1:8080".to_string(), Duration::from_secs(5));
        
        assert!(result.status.is_ok());
        assert_eq!(result.url, "http://127.0.0.1:8080");
        assert!(result.response_time < Duration::from_secs(5));

        // Verify it finished within reasonable time
        assert!(Instant::now().duration_since(start) < Duration::from_secs(10));
    }

    #[test]
    fn test_performance_multiple_concurrent_requests() {
        let urls = vec![
            "http://127.0.0.1:8080".to_string(),
            "http://127.0.0.1:8080".to_string(),
            "http://127.0.0.1:8080".to_string(),
        ];
        let worker_num = 4;
        let timeout = Duration::from_secs(5);
        let retries = 3;

        // start mock server in a separate thread
        mock_server_thread();

        let start = Instant::now();
        let results = monitor_websites(urls, worker_num, timeout, retries);

         // ensures tests complete in time
        assert!(results.len() > 0);
        assert!(Instant::now().duration_since(start) < Duration::from_secs(10));
    }

    #[test]
    fn test_retry_on_failure() {
        let url = "http://127.0.0.1:8080".to_string();
        let timeout = Duration::from_secs(2);
        let retries = 3;

        mock_server_thread();

        let mut result = None;
        for _ in 0..retries {
            result = Some(website_checker(url.clone(), timeout));
            if result.as_ref().unwrap().status.is_ok() {
                break;
            }
        }

        assert!(result.is_some());
        assert!(result.unwrap().status.is_ok());
    }
}

