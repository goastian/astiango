<?php

    if(strpos("https://$_SERVER[HTTP_HOST]$_SERVER[REQUEST_URI]", '?id=') === false){
        include 'landing.php';
    }
    else{
        include 'product.php';
    }

?>

<style>
    *{
    margin: 0;
    padding: 0;
    font-family: sans-serif;
    box-sizing: border-box;
    transition: all 0.3s ease-in-out;
  }
  .header{
    display: grid;
    background-color: #f2f4f7;
    grid-template-columns: auto auto;
    margin: 20px;
    padding: 20px;
    border-radius: 20px;
    height: calc(100vh - 40px);
    width: calc(100vw - 40px);   
  }
  
  .storeItem{
    width: 22%;
    background-color: #f2f4f7;
    padding: 10px;
    text-align: center;
    border-radius: 20px;
    float:left;
    margin:10px;
  }
  .storeItem img{
    width: 100%;
    height: auto;
    border-radius: 20px 20px 0 0;
  }
  </style>

</body>
</html>