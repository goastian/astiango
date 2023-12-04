<html lang="en">

<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>PriEco | Map</title>

    <link rel="stylesheet" href="https://unpkg.com/leaflet@1.9.4/dist/leaflet.css"
        integrity="sha256-p4NxAoJBhIIN+hmNHrzRCf9tD/miZyoHS5obTRR9BMY=" crossorigin="" />
        <script src="https://unpkg.com/leaflet@1.9.4/dist/leaflet.js" integrity="sha256-20nQCchB9co0qIjJZRGuk2/Z9VM+kNiyxNV1lvTlZBo=" crossorigin=""></script>

</head>

<body>
    <div style="height: 100%; width: 100%;">
        <a style="position: absolute;z-index: 999;left: 55px;top: 25px;" href="/?q=<?php echo $_GET['q']?>"><button style="border: none;padding: 10px;border-radius: 20px;cursor:pointer;" class="whiteAblackBg"><</button></a>
        <div class="searchBarMap" tabindex="0">
            <input type="text" id="search-input" class="searchBox" placeholder="Search Map" value="<?php echo $_GET['q'] ?>">
            <button onclick="searchLoc();" class="searchButton"><img alt="icnSearch" src="./View/icon/search.webp" style="width:10px; height:10px;"></button>
            <div id="suggestions-container" class="autocom-box"></div>
        </div>
        <div id="map" style="width: 100%; height: 100%;"></div>
    </div>
    <style>
        * {
            margin: 0;
            padding: 0;
            font-family: sans-serif;
            box-sizing: border-box;
            transition: all 0s ease-in-out !important;
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
        .searchBarMap{
            position: absolute;z-index: 999;left:100px;top:20px;
        }
        #search-input{
            border-radius:20px; width: 350px;height: 50px;
        }
        .searchButton{
            height:50px !important;
            left:-42px !important;
            top:-1px !important;
        }
        #suggestions-container{
            visibility: hidden;
            width: 350px;border-radius: 20px;padding: 10px;font-size: 14px;
        }
        #suggestions-container div:hover{
            background-color: #0001;
        }

        @media (max-width: 890px) {
            .searchBarMap{
                right: 0;
                z-index: 99999;
                display: flex;
                top:0;
                margin-top: -10px;
            }
            #search-input{
                width: 100%;
                margin-left: 0;
            }
            .searchButton{
                left:-30px !important;
                top: 0 !important;
            }
            #suggestions-container{
                width:100%;
            }
        }
    </style>
    <?php
        $cssver = 54;
        include 'Model/style.php';
    ?>

    <script>
        ////
        //Create Map
        ////
        var map;
        function PriEcoMap(x, y, zoom) {
            if (!map) {
                // Initialize the map and marker
                map = L.map('map').setView([x, y], zoom);
                L.tileLayer('https://tile.openstreetmap.org/{z}/{x}/{y}.png', {
                    maxZoom: 19,
                    attribution: '&copy; <a href="http://www.openstreetmap.org/copyright">OpenStreetMap</a>'
                }).addTo(map);
                marker = L.marker([x, y]).addTo(map);
            } else {
                map.setView([x, y], zoom);
                marker.setLatLng([x, y]);
            }
        }
        ////
        //Search locations
        ////
        var inputBox = document.querySelector("#search-input");
        const suggBox = document.querySelector("#suggestions-container");

        inputBox.onfocus = () => {
            const autocompleteUrl = apiUrl + encodeURIComponent(inputBox.value);
                    fetch(autocompleteUrl)
                        .then(response => response.json())
                        .then(data => {
                            handleSuggestions(data);
                        })
                        .catch(error => {
                            console.error('Error:', error);
                        });        }
        inputBox.addEventListener("keypress", function (event) {
            if (event.keyCode === 13) {
                suggBox.style.visibility = "hidden";
                geocodeAddress(inputBox.value);
            }
        });
        inputBox.onblur = () => {
            setTimeout(() => {
                suggBox.style.visibility = "hidden";
            }, 200);

        };
        function searchLoc() {
            inputBox = document.querySelector("#search-input");
            suggBox.style.visibility = "hidden";
            geocodeAddress(inputBox.value);
        }



        const apiUrl = 'https://nominatim.openstreetmap.org/search?format=json&q=';

            // Function to handle autocomplete suggestions
            function handleSuggestions(data) {
                const suggestions = data;
                const suggestionsContainer = document.getElementById('suggestions-container');
                suggestionsContainer.innerHTML = '';
                suggestionsContainer.style.visibility="visible";
                suggestions.forEach(function (suggestion) {
                    const displayName = suggestion.display_name;
                    const suggestionElement = document.createElement('div');
                    suggestionElement.textContent = displayName;
                    suggestionElement.style = 'padding:5px; cursor:pointer; border-radius:20px;';
                    
                    suggestionElement.addEventListener('click', function () {
                        // Update input value with selected suggestion
                        document.getElementById('search-input').value = displayName;
                        // Perform geocoding request
                        geocodeAddress(displayName);
                        // Clear suggestions container
                        suggestionsContainer.innerHTML = '';
                        suggestionsContainer.style.visibility = "hidden";
                    });
                    suggestionsContainer.appendChild(suggestionElement);
                });
            }

            // Function to perform geocoding request
            function geocodeAddress(address) {
                const formattedAddress = encodeURIComponent(address);
                const geocodeUrl = apiUrl + formattedAddress;
                fetch(geocodeUrl)
                    .then(response => response.json())
                    .then(data => {
                        const latitude = data[0].lat;
                        const longitude = data[0].lon;
                        PriEcoMap(latitude, longitude, 18);
                    })
                    .catch(error => {
                        console.error('Error:', error);
                    });
            }

            document.getElementById('search-input').addEventListener('input', function () {
                const searchTerm = this.value;
                if (searchTerm.length >= 3) {
                    const autocompleteUrl = apiUrl + encodeURIComponent(searchTerm);
                    fetch(autocompleteUrl)
                        .then(response => response.json())
                        .then(data => {
                            handleSuggestions(data);
                        })
                        .catch(error => {
                            console.error('Error:', error);
                        });
                }
            });
        





        ////
        //Get cords from location
        ////
        function getCoordinates(location) {
            // Format the location string for the API request
            const formattedLocation = encodeURIComponent(location);

            // Prepare the API request URL
            const apiUrl = `https://nominatim.openstreetmap.org/search?q=${formattedLocation}&format=json`;

            // Send the API request and retrieve the response
            return fetch(apiUrl)
                .then(response => response.json())
                .then(data => {
                    // Parse the response and extract the latitude and longitude
                    const coordinates = {};
                    if (data && data.length > 0) {
                        coordinates.lat = data[0].lat;
                        coordinates.lon = data[0].lon;
                    }
                    return coordinates;
                })
                .catch(error => {
                    console.error('Failed to retrieve coordinates:', error);
                    return null;
                });
        }
        <?php 
        if(isset($_GET['q'])){
            echo 'geocodeAddress("',$_GET['q'],'");';
        }
        else{
            echo 'PriEcoMap(51.5073359, -0.12765, 2);';
        }
        
        ?>
    </script>


</body>

</html>