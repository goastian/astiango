<?php
if(isset($_POST['submitHigh'])){
   
    $targetDir = 'imgs/';
    $targetFile = $targetDir . basename($_FILES["imgfile"]["name"]);
    // Check if the file was successfully uploaded
    if (move_uploaded_file($_FILES["imgfile"]["tmp_name"], $targetFile)) {
        file_put_contents('addH.txt', $_POST['to'].';'.$_POST['name'].';'.$_FILES["imgfile"]["name"].';'.$_POST['link']."\n", FILE_APPEND);
        echo 'Thank you, your request has been uploaded!';
    } else {
        echo "Sorry, there was an error uploading your file.";
    }
}
?>

<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>Highlight | PriEco</title>
</head>
<body>
    <form method="POST" action="" enctype="multipart/form-data" style="display: flex;
    flex-direction: column;
    width: 30%;
    text-align: center;
    ">
        <h1>Suggest private alternative highlight</h1>
        <input type="text" name="to" placeholder="Alternative to: Instagram, Twitter... (If more split by ,)" required>
        <input type="text" name="name" placeholder="Name" required>
        <label for="imgfile">Image:</label><input id="imgfile" name="imgfile" type="file" required></div>
        <input type="url" name="link" placeholder="Link to alternative" required>
        <input type="submit" name="submitHigh" placeholder="Submit">
    </form>

    <style>
        input{
            margin: 5px;
        }
    </style>
</body>
</html>