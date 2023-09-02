# Accounts
```
[
  {
    "id": "bakap-horuv-viror-pozaf-dimon-pizis-fotap-girav",
    "name": "Default Virtual Account",
    "notes": "A virtual account is required to do much, but many transactions don't really need one, so this is a default to use",
    "typ": "Virtual",
    "current": {
      "EUR": "185 EUR",
      "USD": "9 USD"
    },
    "enabled": true
  },
  {
    "id": "bakap-horuv-zasup-bikif-rilif-rojus-ripol-posos",
    "name": "A",
    "notes": "",
    "typ": "Physical",
    "current": {},
    "enabled": false
  },
  {
    "id": "bakap-hosab-vodit-lopav-pasuv-giluv-rukip-ribav",
    "name": "A new account",
    "notes": "",
    "typ": "Physical",
    "current": {
      "EUR": "120 EUR",
      "USD": "9 USD"
    },
    "enabled": true
  },
  {
    "id": "bakap-hosad-gohil-jojav-tahin-mojog-ludum-zutit",
    "name": "A new account",
    "notes": "",
    "typ": "Physical",
    "current": {
      "EUR": "65 EUR"
    },
    "enabled": true
  }
]
```
# Commands
## Add Transaction
```
{
  "AddTransaction": {
    "id": "bakap-jimad-zopob-tasuk-vasud-fakoj-hotuh-fosag",
    "notes": "",
    "amount": "100 EUR",
    "type": "Received",
    "src": "A source",
    "dst": "bakap-hosab-vodit-lopav-pasuv-giluv-rukip-ribav",
    "dst_virt": "bakap-horuv-viror-pozaf-dimon-pizis-fotap-girav"
  }
}
```

## Create Account
```
{
  "CreateAccount": {
    "id": "bakap-jimap-sigis-nudoj-vibab-rimak-mosig-nijam",
    "name": "New Account #3",
    "notes": "",
    "typ": "Virtual",
    "current": {},
    "enabled": true
  }
}
```

## Disable Account
```
{
  "UpdateAccount": [
    "bakap-jimap-sigis-nudoj-vibab-rimak-mosig-nijam",
    [
      "Disable"
    ]
  ]
}

```