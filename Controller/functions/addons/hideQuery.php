<?php
function hideQuery($query){
    $hQuery = '<div class="answer hQuery" id="answer">
<p>Your search query: </p>
<p ';
if(!isset($_COOKIE['dCopy'])){
    $hQuery .= 'onclick="copyQue()"';
}
$hQuery .= 'class="copyQuery" style="cursor:pointer;">https://'. $_SERVER['SERVER_NAME'] .'/?q='.$query.'</p></div>';

return $hQuery;
}