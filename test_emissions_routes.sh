#!/bin/bash

# Script de test pour les routes d'émissions
# Usage: ./test_emissions_routes.sh [netuid] [challenge_id] [mechanism_id]

set -e

API_URL="${API_URL:-http://localhost:15000}"
NETUID="${1:-100}"
CHALLENGE_ID="${2:-}"
MECHANISM_ID="${3:-}"

echo "=========================================="
echo "Test des routes d'émissions"
echo "=========================================="
echo "API URL: $API_URL"
echo "NetUID: $NETUID"
echo ""

# Test 1: Health check
echo "1. Test health check..."
HEALTH=$(curl -s -o /dev/null -w "%{http_code}" "$API_URL/health" || echo "000")
if [ "$HEALTH" = "200" ]; then
    echo "   ✓ Health check OK"
else
    echo "   ✗ Health check FAILED (code: $HEALTH)"
    echo "   Vérifiez que le service est démarré: docker-compose up -d"
    exit 1
fi
echo ""

# Test 2: Subnet emissions
echo "2. Test GET /emissions/subnet/$NETUID..."
SUBNET_RESPONSE=$(curl -s -w "\n%{http_code}" "$API_URL/emissions/subnet/$NETUID")
HTTP_CODE=$(echo "$SUBNET_RESPONSE" | tail -n1)
BODY=$(echo "$SUBNET_RESPONSE" | sed '$d')

if [ "$HTTP_CODE" = "200" ]; then
    echo "   ✓ Route subnet emissions OK"
    echo "   Réponse:"
    echo "$BODY" | jq '.' 2>/dev/null || echo "$BODY" | head -20
    TOTAL_TAO=$(echo "$BODY" | jq -r '.total_daily_emissions_tao' 2>/dev/null || echo "N/A")
    echo "   Total daily emissions: $TOTAL_TAO TAO/day"
else
    echo "   ✗ Route subnet emissions FAILED (code: $HTTP_CODE)"
    echo "   Réponse: $BODY"
fi
echo ""

# Test 3: Mechanisms emissions
echo "3. Test GET /emissions/subnet/$NETUID/mechanisms..."
MECHANISMS_RESPONSE=$(curl -s -w "\n%{http_code}" "$API_URL/emissions/subnet/$NETUID/mechanisms")
HTTP_CODE=$(echo "$MECHANISMS_RESPONSE" | tail -n1)
BODY=$(echo "$MECHANISMS_RESPONSE" | sed '$d')

if [ "$HTTP_CODE" = "200" ]; then
    echo "   ✓ Route mechanisms emissions OK"
    MECH_COUNT=$(echo "$BODY" | jq '. | length' 2>/dev/null || echo "0")
    echo "   Nombre de mécanismes: $MECH_COUNT"
    if [ "$MECH_COUNT" -gt 0 ]; then
        echo "   Premier mécanisme:"
        echo "$BODY" | jq '.[0]' 2>/dev/null || echo "$BODY" | head -10
    fi
else
    echo "   ✗ Route mechanisms emissions FAILED (code: $HTTP_CODE)"
    echo "   Réponse: $BODY"
fi
echo ""

# Test 4: Specific mechanism emissions (si mechanism_id fourni)
if [ -n "$MECHANISM_ID" ]; then
    echo "4. Test GET /api/emissions/subnet/$NETUID/mechanisms/$MECHANISM_ID..."
    MECH_RESPONSE=$(curl -s -w "\n%{http_code}" "$API_URL/api/emissions/subnet/$NETUID/mechanisms/$MECHANISM_ID")
    HTTP_CODE=$(echo "$MECH_RESPONSE" | tail -n1)
    BODY=$(echo "$MECH_RESPONSE" | sed '$d')
    
    if [ "$HTTP_CODE" = "200" ]; then
        echo "   ✓ Route mechanism emissions OK"
        echo "   Réponse:"
        echo "$BODY" | jq '.' 2>/dev/null || echo "$BODY" | head -20
    else
        echo "   ✗ Route mechanism emissions FAILED (code: $HTTP_CODE)"
        echo "   Réponse: $BODY"
    fi
    echo ""
fi

# Test 5: Specific challenge emissions (si challenge_id fourni)
if [ -n "$CHALLENGE_ID" ]; then
    echo "5. Test GET /api/emissions/subnet/$NETUID/challenges/$CHALLENGE_ID..."
    CHALLENGE_RESPONSE=$(curl -s -w "\n%{http_code}" "$API_URL/api/emissions/subnet/$NETUID/challenges/$CHALLENGE_ID")
    HTTP_CODE=$(echo "$CHALLENGE_RESPONSE" | tail -n1)
    BODY=$(echo "$CHALLENGE_RESPONSE" | sed '$d')
    
    if [ "$HTTP_CODE" = "200" ]; then
        echo "   ✓ Route challenge emissions OK"
        echo "   Réponse:"
        echo "$BODY" | jq '.' 2>/dev/null || echo "$BODY" | head -20
    else
        echo "   ✗ Route challenge emissions FAILED (code: $HTTP_CODE)"
        echo "   Réponse: $BODY"
    fi
    echo ""
fi

# Test 6: Vérifier que BittensorService est initialisé
echo "6. Vérification du service Bittensor..."
# On peut vérifier dans les logs ou via une route de debug si elle existe
echo "   (Vérifiez les logs pour confirmer l'initialisation du BittensorService)"
echo ""

echo "=========================================="
echo "Tests terminés"
echo "=========================================="
echo ""
echo "Pour voir les logs: docker-compose logs -f platform-api"
echo "Pour redémarrer: docker-compose restart platform-api"

