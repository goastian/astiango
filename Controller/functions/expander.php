<?php
$api_path = $_GET['sx-api-path'] ?? null;
if (!$api_path) {
    die('No sx-api-path set in query string');
}

$dev = false;

// Create SX API request URL:
$api_url = 'https://api.searchexpander.com' . $api_path;

// Get the JSON-encoded POST body sent by the sxpr() JS function:
$request_data = json_decode(file_get_contents('php://input'), true);

// In this example, API key and domain are stored in the environment variables SX_API_KEY and SX_SE_DOMAIN:
$request_data['key'] = 'md2H5xww9DcpK2pAsj4mRMbTYFUXlhAX'; // Add API key to the SX API request data
$request_data['domain'] = 'jojoyou.org'; // Add search engine domain name to the SX API request data

// Prepare cURL request to the SX API:
$ch = curl_init($api_url);
curl_setopt($ch, CURLOPT_POST, true);
curl_setopt($ch, CURLOPT_RETURNTRANSFER, true);
curl_setopt($ch, CURLOPT_POSTFIELDS, json_encode($request_data));
curl_setopt($ch, CURLOPT_HTTPHEADER, ['Content-Type: application/json']);

// Get the JSON response from the SX API:
$response = curl_exec($ch);

// Check for an error:
$error = curl_error($ch);

curl_close($ch);

if ($error) {
    die('There was an error: '.$error);
}

// Output the JSON response from Search Expander:
echo $response;