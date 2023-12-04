<?php

function printimg($url) {
    $ch = curl_init($url);
    curl_setopt_array($ch, array(
        CURLOPT_RETURNTRANSFER => true,
        CURLOPT_ENCODING => "",
        CURLOPT_USERAGENT => "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/107.0.0.0 Safari/537.36",
        CURLOPT_IPRESOLVE => CURL_IPRESOLVE_V4,
        CURLOPT_CUSTOMREQUEST => "GET",
        CURLOPT_PROTOCOLS => CURLPROTO_HTTPS | CURLPROTO_HTTP,
        CURLOPT_REDIR_PROTOCOLS => CURLPROTO_HTTPS | CURLPROTO_HTTP,
        CURLOPT_MAXREDIRS => 3,
        CURLOPT_TIMEOUT => 8,
        CURLOPT_VERBOSE => false,
        CURLOPT_FOLLOWLOCATION => true,
        CURLOPT_HEADER => 0,
    ));

    $response = curl_exec($ch);
    $content_type = curl_getinfo($ch, CURLINFO_CONTENT_TYPE);
    $http_code = curl_getinfo($ch, CURLINFO_HTTP_CODE);
    curl_close($ch);

    if ($http_code === 200) {
       $image_type = exif_imagetype('data://image/jpeg;base64,' . base64_encode($response));
       $mime_type = image_type_to_mime_type($image_type);

       header("Content-Type: $mime_type");

       $image = imagecreatefromstring($response);

       if ($image !== false) {
           $originalWidth = imagesx($image);
           $originalHeight = imagesy($image);
           $newWidth = 200;
           $newHeight = ($originalHeight / $originalWidth) * $newWidth;
           $newImage = imagecreatetruecolor($newWidth, $newHeight);

           imagecopyresampled($newImage, $image, 0, 0, 0, 0, $newWidth, $newHeight, $originalWidth, $originalHeight);
           imagejpeg($newImage);
           imagedestroy($image);
       }
    }
}

if (isset($_GET['q'])) {
    $url = urldecode($_GET['q']);
    printimg($url);
}