<?php

   if(isset($_POST['anaaccept'])){
    setcookie('userid', rand(1000000, 1000000000000), time() + (86400 * 364), '/');
    header('refresh:0');
    exit();
   }
   elseif(isset($_POST['anadeny'])){
    setcookie('noanalytics', 'true', time() + (86400 * 364), '/');
    header('refresh:0');
    exit();
   }

elseif(!isset($_COOKIE['userid']) && !isset($_COOKIE['noanalytics'])){
   
   echo '
   
        <form action="" method="post" class="analytics">
        <p style="float:left;"><b>Anonymous analytics</b>
        We use anonymous analytics to improve PriEco. Your data is always anonymous and helps us improve PriEco.
        Read more in our <a href="https://jojoyou.org/legal/" target="_blank">Privacy policy</a></p>
        <div>
        <input type="submit" name="anaaccept" value="Accept"style="background-color: #67c667;
        border: none;
        border-radius: 20px;
        font-weight: bold;
        cursor: pointer;
        padding: 10px;" >
        <input type="submit" name="anadeny" value="Deny" style="background-color: #ef5a5a;
        border: none;
        border-radius: 20px;
        font-weight: bold;
        cursor: pointer;
        padding: 10px;">
        </div>
        </form>   
   ';
}


if(isset($_COOKIE['userid'])){
    $usrid = $_COOKIE['userid'];

#Daily
if(!isset($_COOKIE['userid'])){
    setcookie('userid', $usrid,  time() + (86400 * 364), '/');
}

$date = date('Y-m-d');
if($date != '0000-00-00'){
$sql = "SELECT `count` FROM `users` WHERE `day` = '$date'";
$result = $conn->query($sql);

if ($result->num_rows > 0) {
    $row = $result->fetch_assoc();
    $dayData = explode(' ', $row['count']);
    if (!in_array($usrid, $dayData)) {
        $tmp = implode(' ', $dayData).' '. $usrid;
        $sql = "UPDATE users SET count = '$tmp' WHERE `day` = '$date'";
        $conn->query($sql);
    }
} else {
    $sql = "INSERT INTO users (day, count) VALUES ('$date', '$usrid')";
    $conn->query($sql);
}
}

}