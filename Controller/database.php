<?php
//SPDX-FileCopyrightText: 2022, 2022-2022 Roman  Láncoš <jojoyou@jojoyou.org>
//
//SPDX-License-Identifier: AGPL-3.0-or-later


//Database login information
include_once 'envFunc.php';
    use DevCoder\DotEnv;
    (new DotEnv(__DIR__ . '/.env'))->load();

    if(!$dev){
    if(isset($_COOKIE['auth'])){$auth = $_COOKIE['auth'];}
    else{$auth = null;} 
if($_ENV['HOST_NAME'] != "" or $_ENV['AUTH_NAME'] != "" or $_ENV['DATABASE_PASS'] != "" or $_ENV['DATABASE_NAME'] != ""){
   $conn=new mysqli($_ENV['HOST_NAME'],$_ENV['AUTH_NAME'],$_ENV['DATABASE_PASS'],$_ENV['DATABASE_NAME']);
   $conn->set_charset("utf8");
}
}