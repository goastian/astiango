<?php
function cutString($inputString, $len) {
    if (strlen($inputString) <= $len) {return $inputString;}
  
    $trimmed = substr($inputString, 0, $len);
    $lastSpacePos = strrpos($trimmed, ' ');
  
    if ($lastSpacePos === false) {return $trimmed.'...';}
  
    return substr($trimmed, 0, $lastSpacePos).'...';
  }
