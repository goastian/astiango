  <script>
    function debounce(func, wait) {
  let timeout;
  return function() {
    const context = this;
    const args = arguments;
    clearTimeout(timeout);
    timeout = setTimeout(function() {
      func.apply(context, args);
    }, wait);
  };
}

    <?php 
      if (!isset($_COOKIE['sugProvider'])){echo 'const sugUrl ="Controller/functions/addons/sug.php/?t=d&q=";';}
      if (isset($_COOKIE['sugProvider']) && $_COOKIE['sugProvider'] == 'g'){echo 'const sugUrl ="Controller/functions/addons/sug.php/?t=g&q=";';}
      if (isset($_COOKIE['sugProvider']) && $_COOKIE['sugProvider'] == 'p'){echo 'const sugUrl ="Controller/functions/addons/sug.php/?t=p&q=";';}
    ?> 
  // Getting all required elements
const searchWrapper = document.querySelector(".autocomplete");
const inputBox = searchWrapper.querySelector(".searchBox");
const suggBox = searchWrapper.querySelector(".autocom-box");

let backUpInp = "";

// Function to fetch suggestions from the server
async function fetchSuggestions(userData) {
  const response = await fetch(
    sugUrl + userData
  );
  let data = await response.json();

  let emptyArray = [];

  // Process the data to generate the suggestion list
  <?php
  if(!isset($_COOKIE['sugProvider'])){
    echo '
    data = JSON.parse(data);
    data.forEach((item) => {
      if (Array.isArray(item)) {
        item.slice(0, 10).forEach((element) => {
          emptyArray.push(
            `<li class="suggestion" onclick="select(this)"><img loading="lazy" src="/View/icon/search.webp">${element}</li>`
          );
        });
      }
    });';
  }
  elseif($_COOKIE['sugProvider'] == 'g'){
    echo 'data.slice(0, 10).forEach((element) => {
      emptyArray.push(
        `<li class="suggestion" onclick="select(this)"><img loading="lazy" src="/View/icon/search.webp">${element}</li>`
      );
    });';
  }
  elseif (isset($_COOKIE['sugProvider']) && $_COOKIE['sugProvider'] == 'p'){
    echo '    
      data.forEach((item) => {
        if (Array.isArray(item)) {
          item.slice(0, 10).forEach((element) => {
            var sug =  `<li class="suggestion"`;
            if (element.img !== "") { sug += `style="height:70px;"`;}
            sug = sug + `onclick="select(this)">`;
            if (element.img !== "") {
              sug = sug + `<img loading="lazy" style="width:auto; height:60px;border-radius:10px;max-width:100px;filter:unset;" src="/Controller/functions/proxy.php?q=${element.img}">`;
            } else {
              sug = sug + `<img loading="lazy" src="/View/icon/search.webp">`;
            }
            sug = sug + `<p>${element.name}</p></li>`;
    
            emptyArray.push(sug);
          });
        }
      });
    ';
  }
?>
  // Show the suggestions
  showSuggestions(emptyArray);
}

// Function to show the suggestions in the suggestion box
function showSuggestions(list) {
  let listData;
  if (!list.length) {
    const userValue = inputBox.value;
    listData = `<li class="suggestion" onclick="select(this)"><img loading="lazy" src="/View/icon/search.webp">${userValue}</li>`;
  } else {
    listData = list.join("");
  }
  suggBox.innerHTML = listData;
}

const debouncedFetchSuggestions = debounce(fetchSuggestions, 300);

// Event handler when the user releases a key in the input box
inputBox.onkeyup = (e) => {
  const keyCode = e.keyCode;
  const userData = e.target.value.trim();

  if (userData) {
    if (keyCode === 40 || keyCode === 38) {
      // Handle arrow key navigation
      const active = suggBox.querySelector(".active");
      if (active) {
        const next = keyCode === 40 ? active.nextElementSibling : active.previousElementSibling;
        if (next) {
          active.classList.remove("active");
          next.classList.add("active");
        } else {
          setTimeout(() => {
            inputBox.value = backUpInp;
          }, 200);
        }
      } else {
        const firstElement = keyCode === 40 ? suggBox.firstElementChild : suggBox.lastElementChild;
        firstElement.classList.add("active");
      }
      inputBox.value = suggBox.querySelector(".active").textContent.replace('<img loading="lazy" src="/View/icon/search.webp">', '');
    } else if (keyCode === 13) {
      // Handle enter key to submit the form
      const form = document.getElementById("searchForm");
      form.submit();
    } else {
      // Fetch suggestions from the server
      backUpInp = inputBox.value;
      fetchSuggestions(userData);
    }
  } else {
    // Delayed clearing of suggestion box
    setTimeout(() => {
      suggBox.innerHTML = "";
    }, 400);
  }
};

// Event handler when the input box gets focus
inputBox.onfocus = () => {
  const inputValue = inputBox.value.trim();
  if (inputValue) {
    backUpInp = inputValue;
    fetchSuggestions(inputValue);
  }
};

// Event handler when the input box loses focus
inputBox.onblur = () => {
  setTimeout(() => {
    suggBox.innerHTML = "";
  }, 200);
};

// Event handler when a suggestion is selected
function select(element) {
  const selectData = element.textContent;
  inputBox.value = selectData;
  const form = document.getElementById("searchForm");
  form.submit();
}
</script>
</script>