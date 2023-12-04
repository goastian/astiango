<?php
$id = explode('?id=', "https://$_SERVER[HTTP_HOST]$_SERVER[REQUEST_URI]")[1];

?>

<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta http-equiv="X-UA-Compatible" content="IE=edge">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>Product | PriEco</title>
</head>
<body>
    <div style=" display: grid;
    background-color: #f2f4f7;
    grid-template-columns: auto auto;
    margin: 20px;
    padding: 20px;
    border-radius: 20px;
    height: calc(100% - 40px);
    width: calc(100% - 40px);  ">
        <div>
            <h2>Rainbow theme</h2>
            <p>This theme is rainbowy!</p>    
            <button>Buy it</button>
        </div>
    </div>
</body>
</html>