<?php
$query = isset($_GET['q']) ? $_GET['q'] : '';
if(!isset($_GET['t'])){exit();}

if($_GET['t'] == 'p'){
include '../../database.php';

if ($conn->connect_error) {
    die("Connection failed: " . $conn->connect_error);
}

$result = $conn->query("SELECT * FROM `suggestions` WHERE MATCH(name) AGAINST('$query*')");

$suggestions = [];
if ($result->num_rows > 0) {
    while ($row = $result->fetch_assoc()) {
        $suggestions[] = [
            'name' => $row['name'],
            'img' => $row['img']
        ];
    }
}

$similarityScores = [];
foreach ($suggestions as $suggestion) {
    if (substr($suggestion['name'], 0, 1) === "!" && substr($query, 0, 1) !== "!") {
        continue;
    }
    if (strlen($suggestion['name']) < strlen($query)) {
        continue;
    }

    $similarityScores[$suggestion['name']] = similar_text($query, $suggestion['name']) + (10 - strlen($suggestion['name']));
}

arsort($similarityScores);

$topSuggestions = array_keys(array_slice($similarityScores, 0, 6));
$sortedSuggestions = [];
foreach ($topSuggestions as $suggestion) {
    foreach ($suggestions as $item) {
        if ($item['name'] === $suggestion) {
            $sortedSuggestions[] = $item;
            break;
        }
    }
}

$response = [
    0 => $query,
    1 => $sortedSuggestions
];
$conn->close();
}
elseif($_GET['t'] == 'd'){
   $query = str_replace(' ', '+', $query);
   $ch = curl_init('https://ac.duckduckgo.com/ac/?type=list&callback=jsonCallback&_=1600956892202&q='.$query);
   curl_setopt($ch, CURLOPT_RETURNTRANSFER, true);
   curl_setopt($ch, CURLOPT_USERAGENT, 'Mozilla/5.0 (Windows; U; Windows NT 5.1; en-US) AppleWebKit/525.13 (KHTML, like Gecko) Chrome/0.A.B.C Safari/525.13');
   curl_setopt($ch, CURLOPT_TIMEOUT, 3);
   $response = curl_exec($ch);
   curl_close($ch);
}
elseif($_GET['t'] == 'g'){
   $query = str_replace(' ', '+', $query);
   $ch = curl_init('https://www.google.com/complete/search?client=firefox&q='.$query);
   curl_setopt($ch, CURLOPT_RETURNTRANSFER, true);
   curl_setopt($ch, CURLOPT_USERAGENT, 'Mozilla/5.0 (Windows; U; Windows NT 5.1; en-US) AppleWebKit/525.13 (KHTML, like Gecko) Chrome/0.A.B.C Safari/525.13');
   curl_setopt($ch, CURLOPT_TIMEOUT, 3);
   $response = json_decode(curl_exec($ch),true)[1];
   curl_close($ch);
}
header('Content-Type: application/json');
echo json_encode($response);