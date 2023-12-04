<?php

include '../../../Model/style.php';

$ana = true;
include '../../database.php';
$sql = "SELECT * FROM `users`";
$result = $conn->query($sql);
if ($result->num_rows > 0) {
    while ($row = $result->fetch_assoc()) {
        $days[] = $row['day'];
        $counts[] = count(explode(' ',$row['count']));
        $searches[] = $row['searches'];
    }

}




if(isset($_GET['t'])){
    header('Content-Type: application/json; charset=utf-8');
    echo json_encode($ar);
    }
    else{
$maxValue = max($counts);
$scaleFactor = 100 / $maxValue;

echo '<h1 style="text-align:center;">PriEco Statics</h1>
<h2>Daily active users</h2>';
echo '<div class="graph">';
$i=0;
foreach ($counts as $value) {
    $height = $value * $scaleFactor;
    echo '<div class="bar" style="height: ' . $height . '%;"><p>Date: ',$days[$i],'<br>Count: ',$value,'<br>Searches: ',$searches[$i],'</p></div>';
    ++$i;
}
echo '</div>';
    }
?>

<style>
    * {
  margin: 0;
  padding: 0;
  font-family: sans-serif;
  box-sizing: border-box;
  transition: all 0.3s ease-in-out;
}

html,
body {
  max-width: 100%;
  overflow-x: hidden;
  transition: none;
}

img {
  object-fit: contain;
}
.graph {
    display: flex;
    background-color:#0001;
width: 100%;
height: 300px;
padding: 5px;
align-items: flex-end;
overflow: auto;
}

.bar {
    width: 200px;background-color: #1ece56;
color: white;
padding: 10px;
border-radius: 20px;
margin-right: 10px;
max-height: 95%;
}

    </style>