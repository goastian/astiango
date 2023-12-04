<?php

$tmp = explode(" ", $purl);

$dictionary = explode("\n", file_get_contents('Controller/value/dir.txt'));
$fix_AR = '';
$fix = false;

foreach($tmp as &$user_query){
// Find closest match to user's query in dictionary
$closest_match = null;
$closest_distance = PHP_INT_MAX;
foreach ($dictionary as $word) {
    $distance = levenshtein($user_query, $word);
    if ($distance < $closest_distance) {
        $closest_match = $word;
        $closest_distance = $distance;
    }
}

// Check if closest match is significantly different from user's query
if ($closest_distance >= 1) {
    $fix_AR .= ' '.$closest_match;
    $fix = true;
}
else{
    $fix_AR .= ' '. $user_query;
}
}

if($fix){
    $fix_AR = trim($fix_AR);
    echo '<div class="output" style="border-radius: 20px; margin-bottom: 10px; padding-top:10px;"><p>Did you mean: <a href="/?q=' . $fix_AR . '" style="cursor:pointer;">' . $fix_AR . '</a></p></div>';
}