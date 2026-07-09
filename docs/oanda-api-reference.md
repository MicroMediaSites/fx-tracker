# OANDA API Reference

## Authentication

- Header: `Authorization: Bearer <token>`
- Practice: `https://api-fxpractice.oanda.com`
- Live: `https://api-fxtrade.oanda.com`

---

## Endpoints

### Account & Positions

```
GET  /v3/accounts/{accountID}              # Account summary
GET  /v3/accounts/{accountID}/positions    # All positions
GET  /v3/accounts/{accountID}/openPositions # Open positions only
```

### Trading

```
POST /v3/accounts/{accountID}/orders                    # Place order
GET  /v3/accounts/{accountID}/orders                    # List pending orders
PUT  /v3/accounts/{accountID}/orders/{orderID}          # Modify order
PUT  /v3/accounts/{accountID}/trades/{tradeID}/close    # Close trade
```

> Note: DELETE on orders returns 405 - use PUT with cancel instead

### History

```
GET  /v3/accounts/{accountID}/trades        # Trade history
GET  /v3/accounts/{accountID}/transactions  # Transaction log
```

### Market Data

```
GET  /v3/instruments/{instrument}/candles          # Historical candles
GET  /v3/accounts/{accountID}/pricing              # Current prices (REST)
GET  /v3/accounts/{accountID}/pricing/stream       # Price stream (SSE)
```

---

## Data Structures

### Account

```json
{
  "account": {
    "id": "101-001-1234567-001",
    "balance": "10000.0000",
    "unrealizedPL": "150.0000",
    "pl": "500.0000",
    "openTradeCount": 2,
    "marginUsed": "500.0000",
    "marginAvailable": "9500.0000"
  }
}
```

### Position

```json
{
  "instrument": "EUR_USD",
  "long": {
    "units": "10000",
    "averagePrice": "1.08500",
    "unrealizedPL": "75.0000"
  },
  "short": {
    "units": "0"
  }
}
```

### Order (Market)

```json
{
  "order": {
    "type": "MARKET",
    "instrument": "EUR_USD",
    "units": "10000",
    "timeInForce": "FOK",
    "positionFill": "DEFAULT"
  }
}
```
