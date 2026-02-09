// ============================================================================
// Simple Cache Example
// ============================================================================
//
// This is a simplified implementation for demonstration purposes.
// It showcases basic caching concepts with TTL (Time To Live) support.
//
// ============================================================================

use std::collections::HashMap;
use std::time::{Duration, Instant};
use std::thread::sleep;

struct CacheEntry {
    value: String,
    inserted_at: Instant,
    ttl: Duration,
}

struct Cache {
    // create a hashmap that takes the key and cache entry as arguments
    data: HashMap<String, CacheEntry>
}

impl Cache {

    fn new() -> Self {
        Cache {
            // initialize a new hashmap
            data: HashMap::new(),
        }
    }
    
    fn set(&mut self, key: String, value: String, ttl_seconds: u64) {

        // create instance of CacheEntry and set fields values of fn set arguments
        let cache_entry = CacheEntry {
            value: value,
            inserted_at: Instant::now(), // time at insertion using Instant
            ttl: Duration::new(ttl_seconds, 0) // specified time to live using Duration
        }; 

        // insert key and all of cache entry into hashmap
        self.data.insert(key, cache_entry);

    }
    
    fn get(&self, key: &str) -> Option<&String> {

        // get entry from hashmap using key
        let entry = self.data.get(key)?;

        // if key is expired return none, else return the value of key
        if entry.inserted_at.elapsed() > entry.ttl {
            return None
        }

        // return &entry because signature of function return a Option<&String>,
        // we do this to just copy the reference of the data,
        // this is to avoid copying actual data from cache for memory management and latency
        Some(&entry.value)

    }
    
    fn cleanup(&mut self) {

        // use the retain function on the hashmap and check if key is expired
        self.data.retain(|_key, entry| {
            entry.inserted_at.elapsed() <= entry.ttl
        })

    }

}

fn main() {

    let mut cache = Cache::new();
    
    println!("Testing cache with 10 second TTL...\n");
    
    // Test 1: Basic set/get
    cache.set("q1".to_string(), "Rust is awesome!".to_string(), 10);
    println!("✓ Stored: question1 -> 'Rust is awesome!' (TTL: 10s)");
    
    // Test 2: Retrieve immediately
    if let Some(value) = cache.get("q1") {
        println!("Retrived {}", value)
    }

    // Test 3: Retrieve key that does not exist
    match cache.get("q2") {
        Some(value) => println!("Found {}", value),
        None => println!("Key not found")
    }

    // Test 4: Checking key after it expired
    sleep(Duration::from_secs(11));
    match cache.get("q1") {
        Some(value) => println!("Key should be expired but got {}", value),
        None => println!("Key expired")
    }

    // Test 5: Multiple keys and cleanup
    cache.set("q2".to_string(), "Rust is cool!".to_string(), 10);
    println!("✓ Stored: q2 -> 'Rust is cool!' (TTL: 10s)");

    cache.set("q3".to_string(), "Rust is great!".to_string(), 10);
    println!("✓ Stored: q3 -> 'Rust is great!' (TTL: 10s)");

    println!("\nWaiting 11 seconds for expiration...");
    sleep(Duration::from_secs(11));

    println!("Running cleanup...");
    cache.cleanup();

    // Check that expired keys are gone
    if cache.get("q2").is_none() && cache.get("q3").is_none() {
        println!("✓ Cleanup removed expired entries");
    }

}